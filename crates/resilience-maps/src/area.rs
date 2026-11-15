//! The area a pack's data covers, in Web Mercator world units, for drawing
//! (shading, the dotted edge, clipping a pack's data) and for asking
//! whether the crosshair is inside it.
//!
//! An area comes either from the pack's coverage polygon (`rs_coverage`,
//! even-odd rule like `profile.rs`) or, for a file without one, from the
//! rectangle of its `bounds` (see `app.rs` for what that rectangle is
//! allowed to mean). Each ring's longitudes are unwrapped from vertex to
//! vertex, so a ring crossing the antimeridian stays one ring; callers try
//! the copies of the world one turn east and west.

use resilience_core::maps::geo::{LatLon, World, project};

use crate::metadata::Bounds;
use crate::pack::Coverage;

/// World units: x from 0 (180° W) eastwards, y from 0 (north) southwards.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub rings: Vec<Vec<World>>,
    /// min x, min y, max x, max y over every ring.
    pub bbox: [f64; 4],
}

fn unwrapped(points: impl Iterator<Item = LatLon>) -> Vec<World> {
    let mut out: Vec<World> = Vec::new();
    for p in points {
        let mut w = project(p);
        if let Some(prev) = out.last() {
            // The shorter way round from the previous vertex.
            w.x -= (w.x - prev.x).round();
        }
        out.push(w);
    }
    out
}

fn bbox(rings: &[Vec<World>]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in rings.iter().flatten() {
        b[0] = b[0].min(p.x);
        b[1] = b[1].min(p.y);
        b[2] = b[2].max(p.x);
        b[3] = b[3].max(p.y);
    }
    b
}

impl Area {
    /// The coverage polygon; `None` if no ring has 3 points.
    pub fn from_coverage(c: &Coverage) -> Option<Area> {
        let rings: Vec<Vec<World>> = c
            .rings
            .iter()
            .map(|r| unwrapped(r.points.iter().map(|m| m.lat_lon())))
            .filter(|r| r.len() >= 3)
            .collect();
        if rings.is_empty() {
            return None;
        }
        let bbox = bbox(&rings);
        Some(Area { rings, bbox })
    }

    /// The rectangle of `bounds` (west < east, as the metadata checks).
    pub fn from_bounds(b: &Bounds) -> Area {
        let nw = project(LatLon::wrapped(b.north, b.west));
        let se = project(LatLon::wrapped(b.south, b.east));
        // `east` may be 180, which wraps to -180: use the width instead.
        let x1 = nw.x + (b.east - b.west) / 360.0;
        let ring = vec![
            World { x: nw.x, y: nw.y },
            World { x: x1, y: nw.y },
            World { x: x1, y: se.y },
            World { x: nw.x, y: se.y },
        ];
        let rings = vec![ring];
        let bbox = bbox(&rings);
        Area { rings, bbox }
    }

    /// Whether `p` is inside (even-odd over every ring), in any copy of
    /// the world.
    pub fn contains(&self, p: LatLon) -> bool {
        let w = project(p);
        [-1.0, 0.0, 1.0].iter().any(|k| {
            let x = w.x + k;
            x >= self.bbox[0]
                && x <= self.bbox[2]
                && w.y >= self.bbox[1]
                && w.y <= self.bbox[3]
                && self.contains_world(x, w.y)
        })
    }

    fn contains_world(&self, x: f64, y: f64) -> bool {
        let mut inside = false;
        for ring in &self.rings {
            let n = ring.len();
            for i in 0..n {
                let a = ring[i];
                let b = ring[(i + 1) % n];
                if (a.y > y) != (b.y > y) {
                    let t = (y - a.y) / (b.y - a.y);
                    if x < a.x + t * (b.x - a.x) {
                        inside = !inside;
                    }
                }
            }
        }
        inside
    }

    /// Whether the bounding box, shifted by `turns` worlds, meets the
    /// rectangle `[x0, x1] x [y0, y1]`.
    pub fn meets(&self, turns: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
        self.bbox[0] + turns <= x1
            && self.bbox[2] + turns >= x0
            && self.bbox[1] <= y1
            && self.bbox[3] >= y0
    }

    /// The centre of the bounding box.
    pub fn centre(&self) -> LatLon {
        resilience_core::maps::geo::unproject(World {
            x: (self.bbox[0] + self.bbox[2]) / 2.0,
            y: (self.bbox[1] + self.bbox[3]) / 2.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::Ring;
    use crate::profile::Micro;

    fn ll(lat: f64, lon: f64) -> LatLon {
        LatLon::new(lat, lon).expect("valid")
    }

    fn ring(points: &[(f64, f64)], hole: bool) -> Ring {
        Ring {
            hole,
            points: points
                .iter()
                .map(|&(lon, lat)| Micro::from_degrees(lon, lat).expect("micro"))
                .collect(),
        }
    }

    #[test]
    fn point_in_polygon_with_a_hole() {
        let c = Coverage {
            rings: vec![
                ring(&[(5.0, 49.0), (7.0, 49.0), (7.0, 51.0), (5.0, 51.0)], false),
                ring(&[(5.5, 49.5), (6.5, 49.5), (6.5, 50.5), (5.5, 50.5)], true),
            ],
        };
        let a = Area::from_coverage(&c).expect("area");
        assert!(a.contains(ll(49.2, 5.2)));
        assert!(!a.contains(ll(50.0, 6.0)), "in the hole");
        assert!(!a.contains(ll(52.0, 6.0)));
        assert!(!a.contains(ll(50.0, 8.0)));
        let c = a.centre();
        assert!((c.lon() - 6.0).abs() < 1e-6 && (c.lat() - 50.0).abs() < 0.1);
    }

    #[test]
    fn rings_across_the_antimeridian_stay_whole() {
        // Fiji-like: 177 E to 178 W.
        let c = Coverage {
            rings: vec![ring(
                &[
                    (177.0, -19.0),
                    (-178.0, -19.0),
                    (-178.0, -16.0),
                    (177.0, -16.0),
                ],
                false,
            )],
        };
        let a = Area::from_coverage(&c).expect("area");
        assert!(a.bbox[2] - a.bbox[0] < 0.02, "{:?}", a.bbox);
        assert!(a.contains(ll(-17.0, 179.5)));
        assert!(a.contains(ll(-17.0, -179.5)));
        assert!(!a.contains(ll(-17.0, 0.0)));
        assert!(!a.contains(ll(-17.0, 170.0)));
    }

    #[test]
    fn bounds_rectangle() {
        let b = Bounds {
            west: 170.0,
            south: -10.0,
            east: 180.0,
            north: 10.0,
        };
        let a = Area::from_bounds(&b);
        assert!(a.contains(ll(0.0, 175.0)));
        assert!(!a.contains(ll(0.0, -175.0)));
        assert!(!a.contains(ll(20.0, 175.0)));
        assert!(Area::from_bounds(&Bounds::WORLD).contains(ll(0.0, -179.9)));
        // Degenerate coverage gives no area.
        assert!(Area::from_coverage(&Coverage { rings: vec![] }).is_none());
    }
}
