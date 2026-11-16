//! Osmosis polygon filter files (`.poly`), as Geofabrik publishes them
//! for each extract: the coverage of a pack (`rs_coverage`).
//!
//! ```text
//! luxembourg          <- the file's name (ignored)
//! 1                   <- a ring; a name starting with '!' is a hole
//!    5.961753E+00   5.017631E+01     <- longitude latitude
//!    ...
//! END                 <- end of the ring
//! !2
//!    ...
//! END
//! END                 <- end of the file
//! ```
//!
//! The parser is strict about the structure and lenient about white
//! space. Rings are stored without their closing vertex and without
//! repeated vertices; a ring with fewer than 3 distinct vertices is an
//! error, and so is anything past the limits of `profile.rs`.

use crate::profile::{self, Micro};

/// Largest `.poly` file read.
pub const MAX_POLY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolyRing {
    pub hole: bool,
    pub points: Vec<Micro>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolyError {
    pub line: usize,
    pub what: &'static str,
}

impl std::fmt::Display for PolyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.what)
    }
}

impl std::error::Error for PolyError {}

/// Parse a `.poly` file.
pub fn parse(text: &str) -> Result<Vec<PolyRing>, PolyError> {
    if text.len() > MAX_POLY_BYTES {
        return Err(PolyError {
            line: 0,
            what: "file too large",
        });
    }
    let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l.trim()));
    let err = |line, what| PolyError { line, what };
    // The first line is the file's name.
    match lines.next() {
        Some((_, name)) if !name.is_empty() => {}
        _ => return Err(err(1, "no name line")),
    }
    let mut rings = Vec::new();
    let mut total = 0usize;
    let mut ended = false;
    while let Some((n, line)) = lines.next() {
        if line.is_empty() {
            continue;
        }
        if ended {
            return Err(err(n, "text after the final END"));
        }
        if line == "END" {
            ended = true;
            continue;
        }
        let hole = line.starts_with('!');
        let mut points: Vec<Micro> = Vec::new();
        let mut closed = false;
        for (n, line) in lines.by_ref() {
            if line.is_empty() {
                continue;
            }
            if line == "END" {
                closed = true;
                break;
            }
            let mut parts = line.split_whitespace();
            let (Some(x), Some(y), None) = (parts.next(), parts.next(), parts.next()) else {
                return Err(err(n, "expected two numbers"));
            };
            let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>()) else {
                return Err(err(n, "not a number"));
            };
            let p = Micro::from_degrees(x, y).ok_or(err(n, "outside the world"))?;
            if points.last() != Some(&p) {
                points.push(p);
            }
            if points.len() > profile::MAX_RING_VERTICES {
                return Err(err(n, "too many points in one ring"));
            }
        }
        if !closed {
            return Err(err(n, "ring without END"));
        }
        if points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        if points.len() < 3 {
            return Err(err(n, "a ring has fewer than 3 points"));
        }
        total += points.len();
        if total > profile::MAX_COVERAGE_VERTICES {
            return Err(err(n, "too many points"));
        }
        rings.push(PolyRing { hole, points });
        if rings.len() > profile::MAX_COVERAGE_RINGS {
            return Err(err(n, "too many rings"));
        }
    }
    if !ended {
        return Err(err(text.lines().count(), "no final END"));
    }
    if rings.is_empty() {
        return Err(err(1, "no rings"));
    }
    if rings.iter().all(|r| r.hole) {
        return Err(err(1, "only holes"));
    }
    Ok(rings)
}

/// Whether `p` is inside the rings (even-odd rule, as the reader decides).
pub fn contains(rings: &[PolyRing], p: Micro) -> bool {
    let (px, py) = (i64::from(p.lon), i64::from(p.lat));
    let mut inside = false;
    for r in rings {
        let n = r.points.len();
        for i in 0..n {
            let a = r.points[i];
            let b = r.points[(i + 1) % n];
            let (ax, ay, bx, by) = (
                i64::from(a.lon),
                i64::from(a.lat),
                i64::from(b.lon),
                i64::from(b.lat),
            );
            if (ay > py) != (by > py) {
                // x of the edge at py, compared without division:
                // px < ax + (py - ay) * (bx - ax) / (by - ay).
                let lhs = (px - ax) * (by - ay);
                let rhs = (py - ay) * (bx - ax);
                if (by > ay && lhs < rhs) || (by < ay && lhs > rhs) {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

/// The largest absolute latitude of the rings, in degrees.
pub fn max_abs_lat(rings: &[PolyRing]) -> f64 {
    rings
        .iter()
        .flat_map(|r| r.points.iter())
        .map(|p| p.lat_degrees().abs())
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUX_LIKE: &str = "luxembourg\n1\n   5.961753E+00   5.017631E+01\n   6.53E+00 4.95E+01\n\t5.73 49.44\n   5.961753E+00   5.017631E+01\nEND\nEND\n";

    fn m(lon: f64, lat: f64) -> Micro {
        Micro::from_degrees(lon, lat).expect("point")
    }

    #[test]
    fn geofabrik_files_parse() {
        let r = parse(LUX_LIKE).expect("poly");
        assert_eq!(r.len(), 1);
        assert!(!r[0].hole);
        // The closing vertex is dropped.
        assert_eq!(
            r[0].points,
            vec![m(5.961753, 50.17631), m(6.53, 49.5), m(5.73, 49.44)]
        );
        assert!((max_abs_lat(&r) - 50.17631).abs() < 1e-9);
    }

    #[test]
    fn several_rings_and_holes() {
        let text = "multi\nfirst\n0 0\n10 0\n10 10\n0 10\nEND\n!hole\n2 2\n2 4\n4 4\n4 2\n2 2\nEND\nsecond\n20 0\n21 0\n21 1\nEND\nEND\n\n";
        let r = parse(text).expect("poly");
        assert_eq!(
            r.iter().map(|x| x.hole).collect::<Vec<_>>(),
            vec![false, true, false]
        );
        assert_eq!(r[1].points.len(), 4);
        assert!(contains(&r, m(1.0, 1.0)));
        assert!(!contains(&r, m(3.0, 3.0)), "inside the hole");
        assert!(contains(&r, m(20.8, 0.5)));
        assert!(!contains(&r, m(15.0, 5.0)));
        assert!(!contains(&r, m(-1.0, 5.0)));
    }

    #[test]
    fn broken_files_are_refused() {
        for (text, what) in [
            ("", "no name line"),
            ("name\n1\n0 0\n1 0\n1 1\nEND\n", "no final END"),
            ("name\n1\n0 0\n1 0\n1 1\n", "ring without END"),
            (
                "name\n1\n0 0\n1 0\nEND\nEND\n",
                "a ring has fewer than 3 points",
            ),
            (
                "name\n1\n0 0\n0 0\n1 1\n0 0\nEND\nEND\n",
                "a ring has fewer than 3 points",
            ),
            (
                "name\n1\n0 0 0\n1 0\n1 1\nEND\nEND\n",
                "expected two numbers",
            ),
            ("name\n1\nx y\n1 0\n1 1\nEND\nEND\n", "not a number"),
            ("name\n1\nNaN 0\n1 0\n1 1\nEND\nEND\n", "outside the world"),
            ("name\n1\n0 91\n1 0\n1 1\nEND\nEND\n", "outside the world"),
            ("name\nEND\n", "no rings"),
            ("name\n!1\n0 0\n1 0\n1 1\nEND\nEND\n", "only holes"),
            (
                "name\n1\n0 0\n1 0\n1 1\nEND\nEND\nmore\n",
                "text after the final END",
            ),
        ] {
            assert_eq!(parse(text).map_err(|e| e.what), Err(what), "{text:?}");
        }
        let mut huge = String::from("name\n1\n");
        for i in 0..=profile::MAX_RING_VERTICES {
            huge.push_str(&format!("{} 0\n", i as f64 * 1e-4));
        }
        huge.push_str("END\nEND\n");
        assert_eq!(
            parse(&huge).map_err(|e| e.what),
            Err("too many points in one ring")
        );
    }
}
