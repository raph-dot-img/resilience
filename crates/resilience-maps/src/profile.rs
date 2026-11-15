//! Map pack profile `resilience-1` (ADR 0016 §1): the exact schema of our
//! own `rs_` tables and the layout of their blobs.
//!
//! The pack builder (`resilience-mapmaker finish`) creates the tables by
//! executing [`TABLES`] verbatim, in order, and the reader uses a table only
//! if `sqlite_schema` holds exactly what these statements produce
//! (including the shadow tables that FTS5 and R*Tree add), so reader and
//! builder cannot drift apart. Changing one byte of a statement is a new
//! profile.
//!
//! The statements avoid `STRICT` so that SQLite older than 3.37 (in other
//! MBTiles readers, which ignore `rs_` tables) can still load the schema.
//!
//! # Units
//!
//! Every coordinate is an integer number of **microdegrees** (degrees x
//! 1,000,000, rounded to the nearest), longitude in [-180,000,000,
//! 180,000,000] and latitude in [-90,000,000, 90,000,000]. That is about
//! 11 cm, as the R*Tree's 32-bit integers require.
//!
//! # `rs_coverage`
//!
//! One row per ring of the area the pack's data covers (from Geofabrik's
//! `.poly` file). `points` is the ring's vertices, each 8 bytes: longitude
//! then latitude, each an `i32` little-endian, in microdegrees. The ring
//! is closed implicitly (the last vertex is not repeated) and has at least
//! 3 vertices. `hole` is 1 for the `.poly` sections whose name starts with
//! `!` and 0 otherwise. Whether a point is covered is decided by the
//! even-odd rule over all rings, so `hole` is informative only.
//!
//! # `rs_place`
//!
//! One searchable row (ADR §4). `kind` is one of [`PLACE_KINDS`]; `class`
//! is the OSM class (`town`, `pharmacy`, `residential`...); `name` the name
//! shown; `fold` the name folded for comparison; `near` the nearest
//! settlement within 10 km or NULL; `lon`, `lat` the position in
//! microdegrees; `rank` orders rows of one kind (lower first); `bbox` is
//! NULL for a point, otherwise 16 bytes: west, south, east, north, each an
//! `i32` little-endian in microdegrees.
//!
//! # `rs_place_fts`, `rs_poi_rtree`
//!
//! The FTS5 index over `rs_place.name` (external content, rowid =
//! `rs_place.id`) and the R*Tree over points of interest and peaks (id =
//! `rs_place.id`; for a point, min = max).

/// Value of the `resilience_profile` metadata key for this profile.
pub const PROFILE_VALUE: &str = "1";
/// The profile's name as shown to people.
pub const PROFILE_NAME: &str = "resilience-1";

pub const RS_COVERAGE: &str = "CREATE TABLE rs_coverage (ring INTEGER PRIMARY KEY, hole INTEGER NOT NULL CHECK (hole IN (0, 1)), points BLOB NOT NULL)";

pub const RS_PLACE: &str = "CREATE TABLE rs_place (id INTEGER PRIMARY KEY, kind TEXT NOT NULL CHECK (kind IN ('settlement', 'street', 'poi', 'natural')), class TEXT NOT NULL, name TEXT NOT NULL, fold TEXT NOT NULL, near TEXT, lon INTEGER NOT NULL, lat INTEGER NOT NULL, rank INTEGER NOT NULL, bbox BLOB)";

pub const RS_PLACE_FTS: &str = "CREATE VIRTUAL TABLE rs_place_fts USING fts5(name, content='rs_place', content_rowid='id', tokenize='unicode61 remove_diacritics 2', prefix='2 3')";

pub const RS_POI_RTREE: &str =
    "CREATE VIRTUAL TABLE rs_poi_rtree USING rtree_i32(id, min_lon, max_lon, min_lat, max_lat)";

/// Every statement of the profile, in the order the builder runs them.
pub const TABLES: [&str; 4] = [RS_COVERAGE, RS_PLACE, RS_PLACE_FTS, RS_POI_RTREE];

/// The values allowed in `rs_place.kind`.
pub const PLACE_KINDS: [&str; 4] = ["settlement", "street", "poi", "natural"];

/// Most rings in `rs_coverage`.
pub const MAX_COVERAGE_RINGS: usize = 1000;
/// Most vertices in one ring (the 4 MiB blob limit allows 524,288).
pub const MAX_RING_VERTICES: usize = 500_000;
/// Most vertices over all rings (8 MB once decoded).
pub const MAX_COVERAGE_VERTICES: usize = 1_000_000;

/// A position in microdegrees, as stored. Always within the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Micro {
    pub lon: i32,
    pub lat: i32,
}

pub const MICRO: f64 = 1_000_000.0;
const MAX_LON: i32 = 180_000_000;
const MAX_LAT: i32 = 90_000_000;

impl Micro {
    /// `None` outside the world.
    pub fn new(lon: i32, lat: i32) -> Option<Micro> {
        ((-MAX_LON..=MAX_LON).contains(&lon) && (-MAX_LAT..=MAX_LAT).contains(&lat))
            .then_some(Micro { lon, lat })
    }

    /// From degrees, rounded; `None` if not finite or outside the world.
    pub fn from_degrees(lon: f64, lat: f64) -> Option<Micro> {
        if !lon.is_finite() || !lat.is_finite() {
            return None;
        }
        let (x, y) = ((lon * MICRO).round(), (lat * MICRO).round());
        if x.abs() > f64::from(MAX_LON) || y.abs() > f64::from(MAX_LAT) {
            return None;
        }
        // In range, so the conversion is exact.
        Micro::new(x as i32, y as i32)
    }

    pub fn lon_degrees(self) -> f64 {
        f64::from(self.lon) / MICRO
    }

    pub fn lat_degrees(self) -> f64 {
        f64::from(self.lat) / MICRO
    }

    pub fn lat_lon(self) -> resilience_core::maps::geo::LatLon {
        resilience_core::maps::geo::LatLon::wrapped(self.lat_degrees(), self.lon_degrees())
    }
}

/// Why a stored blob was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobError {
    /// The length is not a whole number of vertices, or the wrong size.
    Length,
    /// Fewer than 3 vertices, or more than the limit.
    VertexCount,
    /// A coordinate outside the world.
    OutsideWorld,
    /// West after east or south after north.
    Reversed,
}

impl std::fmt::Display for BlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BlobError::Length => "wrong length",
            BlobError::VertexCount => "wrong number of points",
            BlobError::OutsideWorld => "a point outside the world",
            BlobError::Reversed => "west after east or south after north",
        })
    }
}

fn micro_at(b: &[u8], i: usize) -> Result<Micro, BlobError> {
    let w = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    Micro::new(w(i), w(i + 4)).ok_or(BlobError::OutsideWorld)
}

/// Decode a `rs_coverage.points` blob.
pub fn decode_ring(blob: &[u8]) -> Result<Vec<Micro>, BlobError> {
    if !blob.len().is_multiple_of(8) {
        return Err(BlobError::Length);
    }
    let n = blob.len() / 8;
    if !(3..=MAX_RING_VERTICES).contains(&n) {
        return Err(BlobError::VertexCount);
    }
    (0..n).map(|i| micro_at(blob, i * 8)).collect()
}

/// Encode a ring for `rs_coverage.points` (the builder's side).
pub fn encode_ring(points: &[Micro]) -> Vec<u8> {
    let mut out = Vec::with_capacity(points.len() * 8);
    for p in points {
        out.extend_from_slice(&p.lon.to_le_bytes());
        out.extend_from_slice(&p.lat.to_le_bytes());
    }
    out
}

/// A `rs_place.bbox`: south-west and north-east corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicroBox {
    pub south_west: Micro,
    pub north_east: Micro,
}

/// Decode a `rs_place.bbox` blob.
pub fn decode_bbox(blob: &[u8]) -> Result<MicroBox, BlobError> {
    if blob.len() != 16 {
        return Err(BlobError::Length);
    }
    let (sw, ne) = (micro_at(blob, 0)?, micro_at(blob, 8)?);
    if sw.lon > ne.lon || sw.lat > ne.lat {
        return Err(BlobError::Reversed);
    }
    Ok(MicroBox {
        south_west: sw,
        north_east: ne,
    })
}

/// Encode a `rs_place.bbox` blob (the builder's side).
pub fn encode_bbox(b: MicroBox) -> Vec<u8> {
    encode_ring(&[b.south_west, b.north_east])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_round_trip_and_are_bounded() {
        let ring = [
            Micro::new(5_730_000, 49_440_000).expect("should succeed"),
            Micro::new(6_530_000, 49_440_000).expect("should succeed"),
            Micro::new(6_130_000, 50_190_000).expect("should succeed"),
        ];
        assert_eq!(
            decode_ring(&encode_ring(&ring)).expect("should succeed"),
            ring
        );
        assert_eq!(
            decode_ring(&encode_ring(&ring[..2])),
            Err(BlobError::VertexCount)
        );
        assert_eq!(decode_ring(&[0; 25]), Err(BlobError::Length));
        let mut out = encode_ring(&ring);
        out[4..8].copy_from_slice(&90_000_001i32.to_le_bytes());
        assert_eq!(decode_ring(&out), Err(BlobError::OutsideWorld));
        out[4..8].copy_from_slice(&i32::MIN.to_le_bytes());
        assert_eq!(decode_ring(&out), Err(BlobError::OutsideWorld));
        assert_eq!(
            decode_ring(&vec![0; (MAX_RING_VERTICES + 1) * 8]),
            Err(BlobError::VertexCount)
        );
    }

    #[test]
    fn bounding_boxes_round_trip_and_are_checked() {
        let b = MicroBox {
            south_west: Micro::new(-1, -2).expect("should succeed"),
            north_east: Micro::new(3, 4).expect("should succeed"),
        };
        assert_eq!(decode_bbox(&encode_bbox(b)), Ok(b));
        let rev = MicroBox {
            south_west: b.north_east,
            north_east: b.south_west,
        };
        assert_eq!(decode_bbox(&encode_bbox(rev)), Err(BlobError::Reversed));
        assert_eq!(decode_bbox(&[0; 15]), Err(BlobError::Length));
    }

    #[test]
    fn degrees_convert_with_bounds() {
        let m = Micro::from_degrees(6.1319, 49.6116).expect("should succeed");
        assert_eq!((m.lon, m.lat), (6_131_900, 49_611_600));
        assert!((m.lat_degrees() - 49.6116).abs() < 1e-9);
        assert_eq!(
            Micro::from_degrees(180.0, 90.0).map(|m| m.lat),
            Some(90_000_000)
        );
        assert_eq!(Micro::from_degrees(180.000_001, 0.0), None);
        assert_eq!(Micro::from_degrees(f64::NAN, 0.0), None);
        assert_eq!(Micro::from_degrees(0.0, -90.5), None);
    }
}
