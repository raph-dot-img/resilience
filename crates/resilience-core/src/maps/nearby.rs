//! "What is near the crosshair" (ADR 0016 §4, `n`): the boxes asked of
//! each pack's R*Tree, and the order of what comes back. Pure: the
//! queries run in the viewer's reader.
//!
//! The reader asks for points in a box of [`BOXES_M`]`[0]` around the
//! crosshair, then the next box while fewer than [`WANT`] were found. The
//! answer is sorted by distance on a sphere, with each point's category
//! (most urgent first in the filter: health, water, shelter, safety,
//! food...).

use super::geo::{LatLon, distance_m};
use super::search::{Hit, Kind};
use super::style::{PoiCategory, poi_category};

/// Half-sizes of the boxes searched in turn, in metres.
pub const BOXES_M: [f64; 3] = [500.0, 2_000.0, 10_000.0];
/// The reader stops widening once it has this many points.
pub const WANT: usize = 50;
/// Rows read from one pack for one box, at most.
pub const MAX_ROWS: usize = 1000;
/// Points shown, at most.
pub const MAX_SHOWN: usize = 200;

/// The box around `centre` reaching `half_m` metres in every direction,
/// in microdegrees (west, south, east, north), clamped to the world. Near
/// the poles or across the antimeridian it is cut at the edge of the
/// world rather than wrapped (packs there are not expected).
pub fn box_micro(centre: LatLon, half_m: f64) -> (i32, i32, i32, i32) {
    let half = half_m.clamp(1.0, 1_000_000.0);
    let dlat = (half / 111_320.0).min(10.0);
    let cos = centre.lat().to_radians().cos().max(0.01);
    let dlon = (half / (111_320.0 * cos)).min(20.0);
    let micro = |v: f64, max: f64| (v.clamp(-max, max) * 1e6).round() as i32;
    (
        micro(centre.lon() - dlon, 180.0),
        micro(centre.lat() - dlat, 90.0),
        micro(centre.lon() + dlon, 180.0),
        micro(centre.lat() + dlat, 90.0),
    )
}

/// A point near the crosshair.
#[derive(Debug, Clone, PartialEq)]
pub struct Near {
    pub hit: Hit,
    pub category: PoiCategory,
    pub distance_m: f64,
}

/// The category of a row: its class for a point of interest; peaks and
/// other natural points are "other".
pub fn category(h: &Hit) -> PoiCategory {
    match h.kind {
        Kind::Poi => poi_category(&h.class),
        _ => PoiCategory::Other,
    }
}

/// Merge the rows of every pack: points of interest and natural points
/// only, the same place from two packs once, nearest first, at most
/// [`MAX_SHOWN`].
pub fn order(hits: Vec<Hit>, centre: LatLon) -> Vec<Near> {
    let mut v: Vec<Near> = hits
        .into_iter()
        .filter(|h| matches!(h.kind, Kind::Poi | Kind::Natural))
        .map(|h| Near {
            category: category(&h),
            distance_m: distance_m(centre, h.at),
            hit: h,
        })
        .collect();
    v.sort_by(|a, b| {
        a.distance_m
            .partial_cmp(&b.distance_m)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.category.cmp(&b.category))
            .then(a.hit.name.cmp(&b.hit.name))
    });
    let mut out: Vec<Near> = Vec::new();
    for n in v {
        let dup = out.iter().any(|o| {
            o.hit.fold == n.hit.fold
                && o.category == n.category
                && distance_m(o.hit.at, n.hit.at) < 50.0
        });
        if !dup {
            out.push(n);
        }
        if out.len() >= MAX_SHOWN {
            break;
        }
    }
    out
}

/// The categories present, most urgent first (for Tab).
pub fn categories(found: &[Near]) -> Vec<PoiCategory> {
    let mut c: Vec<PoiCategory> = found.iter().map(|n| n.category).collect();
    c.sort();
    c.dedup();
    c
}

/// What a category is called in the list.
pub fn category_name(c: PoiCategory) -> &'static str {
    match c {
        PoiCategory::Health => "health",
        PoiCategory::Water => "water",
        PoiCategory::Shelter => "shelter",
        PoiCategory::Safety => "safety",
        PoiCategory::Food => "food",
        PoiCategory::Fuel => "fuel",
        PoiCategory::Toilets => "toilets",
        PoiCategory::Post => "post",
        PoiCategory::TownHall => "town hall",
        PoiCategory::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maps::search::search_fold;

    fn h(kind: Kind, class: &str, name: &str, lat: f64, lon: f64) -> Hit {
        Hit {
            pack: 0,
            kind,
            class: class.into(),
            name: name.into(),
            fold: search_fold(name),
            near: None,
            at: LatLon::new(lat, lon).expect("in range"),
            rank: 0,
        }
    }

    #[test]
    fn boxes_grow_and_stay_in_the_world() {
        let c = LatLon::new(49.6, 6.1).expect("c");
        let (w, s, e, n) = box_micro(c, 500.0);
        assert!(w < 6_100_000 && e > 6_100_000 && s < 49_600_000 && n > 49_600_000);
        // About 500 m north and south (4.5 thousandths of a degree).
        assert!((f64::from(n - 49_600_000) - 4491.0).abs() < 5.0, "{n}");
        let big = box_micro(c, BOXES_M[2]);
        assert!(big.0 < w && big.3 > n);
        // Near a pole or the antimeridian: cut, never wrapped or overflowed.
        let p = box_micro(LatLon::new(89.99, 179.99).expect("p"), 10_000.0);
        assert!(p.3 <= 90_000_000 && p.2 <= 180_000_000 && p.0 < p.2 && p.1 < p.3);
        let q = box_micro(LatLon::new(-89.99, -179.99).expect("q"), 10_000.0);
        assert!(q.1 >= -90_000_000 && q.0 >= -180_000_000);
    }

    #[test]
    fn nearest_first_with_categories() {
        let c = LatLon::new(49.6, 6.1).expect("c");
        let got = order(
            vec![
                h(Kind::Poi, "pharmacy", "Far pharmacy", 49.62, 6.1),
                h(Kind::Poi, "drinking_water", "Fountain", 49.601, 6.1),
                h(Kind::Settlement, "town", "Not a point", 49.6, 6.1),
                h(Kind::Natural, "peak", "Hill", 49.605, 6.1),
                h(Kind::Poi, "cafe", "Cafe", 49.6005, 6.1),
            ],
            c,
        );
        let names: Vec<&str> = got.iter().map(|n| n.hit.name.as_str()).collect();
        assert_eq!(names, ["Cafe", "Fountain", "Hill", "Far pharmacy"]);
        assert_eq!(got[1].category, PoiCategory::Water);
        assert_eq!(got[0].category, PoiCategory::Other);
        assert_eq!(got[3].category, PoiCategory::Health);
        assert_eq!(
            categories(&got),
            [PoiCategory::Health, PoiCategory::Water, PoiCategory::Other]
        );
        assert_eq!(category_name(PoiCategory::TownHall), "town hall");
        // The same point from two packs once.
        let a = h(Kind::Poi, "pharmacy", "P", 49.6, 6.1);
        let mut b = a.clone();
        b.pack = 1;
        assert_eq!(order(vec![a, b], c).len(), 1);
    }
}
