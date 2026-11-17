//! Geographic maths for the map viewer (ADR 0016 §3 and §4).
//!
//! - **Web Mercator** ([`project`], [`unproject`]) in *world units*: the
//!   whole map is the unit square, `x` growing east from the antimeridian,
//!   `y` growing south from the projection's northern limit, exactly like
//!   XYZ tile numbers. Latitude is clamped at [`MERCATOR_MAX_LAT`] and
//!   longitude wraps.
//! - **The view** ([`View`]): a centre and a scale in metres per pixel *at
//!   the centre*, so the zoom does not depend on the window size. `+`/`-`
//!   step along [`ZOOM_LADDER`]. A [`Viewport`] is a view at a size in
//!   pixels: it chooses the tile zoom, lists the tiles that cover it
//!   (bounded by [`MAX_VIEW_TILES`]) and gives each tile a
//!   [`TileTransform`] from tile-local coordinates to screen pixels.
//! - **Tile numbers** come in two conventions that must never be mixed:
//!   [`TileId`] is XYZ (row 0 is the north), [`TmsTile`] is the TMS order
//!   that MBTiles stores (row 0 is the south). Only explicit conversions
//!   go from one to the other.
//! - **Distance and direction** on a sphere, human distance text.
//! - **Coordinates** formatted in decimal degrees or degrees, minutes and
//!   seconds, and parsed from what a person types ([`parse_coordinates`]).
//! - **Scale bar** lengths in a 1-2-5 series ([`scale_bar`]).
//!
//! Every input is treated as possibly NaN or infinite: such values are
//! replaced or refused before they can reach an index, a cast or a loop.

use std::f64::consts::PI;
use std::fmt;

/// The latitude where Web Mercator's square ends: `atan(sinh(pi))`.
pub const MERCATOR_MAX_LAT: f64 = 85.051_128_779_806_59;
/// The view's centre stops here when panning or jumping (ADR §3: "latitude
/// stops at ±85°"), a little inside the projection's limit.
pub const VIEW_MAX_LAT: f64 = 85.0;
/// WGS 84 equatorial radius, the radius of the Web Mercator sphere.
pub const EARTH_EQUATORIAL_RADIUS_M: f64 = 6_378_137.0;
/// Length of the equator on the Web Mercator sphere (one world unit there).
pub const EARTH_CIRCUMFERENCE_M: f64 = 2.0 * PI * EARTH_EQUATORIAL_RADIUS_M;
/// Mean Earth radius (IUGG) used for distances on a sphere.
pub const EARTH_MEAN_RADIUS_M: f64 = 6_371_008.8;
/// Pixels across one tile at its nominal zoom, as on web maps.
pub const TILE_PIXELS: f64 = 256.0;
/// Highest tile zoom handled at all (2^24 tiles fits a `u32` with room).
pub const MAX_TILE_ZOOM: u8 = 24;

/// The zoom steps of `+` and `-`, in metres per pixel at the centre.
pub const ZOOM_LADDER: [f64; 15] = [
    5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0,
    50_000.0, 100_000.0, 200_000.0,
];
/// Finest scale of a view, metres per pixel.
pub const MIN_SCALE: f64 = ZOOM_LADDER[0];
/// Coarsest scale of a view, metres per pixel.
pub const MAX_SCALE: f64 = ZOOM_LADDER[ZOOM_LADDER.len() - 1];

/// A viewport is at most this many pixels wide or high (larger sizes are
/// clamped). A 4K screen in an 8x16 console font is 480x270 pixels.
pub const MAX_VIEW_PIXELS: u32 = 1024;
/// A view never asks for more tiles than this (see [`Viewport::tiles`]).
pub const MAX_VIEW_TILES: usize = 64;
/// "Go to coordinates" input longer than this (in characters) is refused.
pub const MAX_COORD_INPUT: usize = 100;

// ---------------------------------------------------------------------------
// Positions and projection
// ---------------------------------------------------------------------------

/// A position in degrees: latitude in [-90, 90], longitude in [-180, 180).
/// Both are always finite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    lat: f64,
    lon: f64,
}

impl LatLon {
    /// A position from exact values: `None` unless both are finite,
    /// `|lat| <= 90` and `|lon| <= 180` (180 is stored as -180).
    pub fn new(lat: f64, lon: f64) -> Option<LatLon> {
        if lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0 {
            Some(LatLon {
                lat,
                lon: wrap_lon(lon),
            })
        } else {
            None
        }
    }

    /// A position from any values: latitude clamped to [-90, 90], longitude
    /// wrapped; a NaN or infinite value becomes 0.
    pub fn wrapped(lat: f64, lon: f64) -> LatLon {
        let lat = if lat.is_finite() {
            lat.clamp(-90.0, 90.0)
        } else {
            0.0
        };
        LatLon {
            lat,
            lon: wrap_lon(lon),
        }
    }

    pub fn lat(self) -> f64 {
        self.lat
    }

    pub fn lon(self) -> f64 {
        self.lon
    }
}

/// Longitude wrapped into [-180, 180); NaN and infinities become 0.
pub fn wrap_lon(lon: f64) -> f64 {
    if !lon.is_finite() {
        return 0.0;
    }
    if (-180.0..180.0).contains(&lon) {
        return lon;
    }
    let w = (lon + 180.0).rem_euclid(360.0) - 180.0;
    // rem_euclid can round up to exactly 360 for tiny negative inputs.
    if w >= 180.0 { -180.0 } else { w }
}

/// A point in Web Mercator world units: `x` in [0, 1) from the antimeridian
/// eastwards, `y` from 0 (north, [`MERCATOR_MAX_LAT`]) to 1 (south). Values
/// outside the square appear in view computations (a view can extend past
/// the poles' limit, or around the world).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct World {
    pub x: f64,
    pub y: f64,
}

/// Web Mercator forward. Latitudes beyond [`MERCATOR_MAX_LAT`] land on the
/// edge of the square.
pub fn project(p: LatLon) -> World {
    let y = if p.lat >= MERCATOR_MAX_LAT {
        0.0
    } else if p.lat <= -MERCATOR_MAX_LAT {
        1.0
    } else {
        (0.5 - p.lat.to_radians().tan().asinh() / (2.0 * PI)).clamp(0.0, 1.0)
    };
    World {
        x: (p.lon + 180.0) / 360.0,
        y,
    }
}

/// Web Mercator inverse. `x` wraps around the world, `y` is clamped to the
/// square; NaN or infinite values are read as the middle of the map.
pub fn unproject(w: World) -> LatLon {
    let x = if w.x.is_finite() { w.x } else { 0.5 };
    let y = if w.y.is_finite() {
        w.y.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let lat = (PI * (1.0 - 2.0 * y)).sinh().atan().to_degrees();
    LatLon::wrapped(lat, x.rem_euclid(1.0) * 360.0 - 180.0)
}

/// Ground metres in one world unit along a parallel at `lat` (degrees).
fn metres_per_world_unit(lat: f64) -> f64 {
    EARTH_CIRCUMFERENCE_M
        * lat
            .clamp(-MERCATOR_MAX_LAT, MERCATOR_MAX_LAT)
            .to_radians()
            .cos()
}

// ---------------------------------------------------------------------------
// Tile numbers
// ---------------------------------------------------------------------------

/// A tile in **XYZ** order (as in web map URLs): row `y = 0` is the
/// northernmost. Always valid: `z <= MAX_TILE_ZOOM`, `x, y < 2^z`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    z: u8,
    x: u32,
    y: u32,
}

/// A tile in **TMS** order, as stored in an MBTiles `tiles` table
/// (`zoom_level`, `tile_column`, `tile_row`): row 0 is the southernmost.
/// Always valid like [`TileId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TmsTile {
    zoom: u8,
    column: u32,
    row: u32,
}

fn tiles_across(z: u8) -> u32 {
    1u32 << z.min(MAX_TILE_ZOOM)
}

impl TileId {
    pub fn new(z: u8, x: u32, y: u32) -> Option<TileId> {
        (z <= MAX_TILE_ZOOM && x < tiles_across(z) && y < tiles_across(z)).then_some(TileId {
            z,
            x,
            y,
        })
    }

    /// The tile at zoom `z` (clamped to [`MAX_TILE_ZOOM`]) holding `p`.
    pub fn containing(p: LatLon, z: u8) -> TileId {
        let z = z.min(MAX_TILE_ZOOM);
        let n = f64::from(tiles_across(z));
        let w = project(p);
        let max = tiles_across(z) - 1;
        // `as` saturates; the world point is finite and inside the square.
        let x = ((w.x * n).floor() as u32).min(max);
        let y = ((w.y * n).floor() as u32).min(max);
        TileId { z, x, y }
    }

    pub fn z(self) -> u8 {
        self.z
    }

    pub fn x(self) -> u32 {
        self.x
    }

    /// Row counted from the north (XYZ).
    pub fn y(self) -> u32 {
        self.y
    }

    /// The same tile in MBTiles order (row flipped).
    pub fn to_tms(self) -> TmsTile {
        TmsTile {
            zoom: self.z,
            column: self.x,
            row: tiles_across(self.z) - 1 - self.y,
        }
    }
}

impl TmsTile {
    pub fn new(zoom: u8, column: u32, row: u32) -> Option<TmsTile> {
        (zoom <= MAX_TILE_ZOOM && column < tiles_across(zoom) && row < tiles_across(zoom))
            .then_some(TmsTile { zoom, column, row })
    }

    pub fn zoom(self) -> u8 {
        self.zoom
    }

    pub fn column(self) -> u32 {
        self.column
    }

    /// Row counted from the south (TMS, MBTiles `tile_row`).
    pub fn row(self) -> u32 {
        self.row
    }

    /// The same tile in XYZ order (row flipped).
    pub fn to_xyz(self) -> TileId {
        TileId {
            z: self.zoom,
            x: self.column,
            y: tiles_across(self.zoom) - 1 - self.row,
        }
    }
}

/// The zooms a pack holds tiles for (MBTiles `minzoom`/`maxzoom`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoomRange {
    min: u8,
    max: u8,
}

impl ZoomRange {
    /// `None` unless `min <= max <= MAX_TILE_ZOOM`.
    pub fn new(min: u8, max: u8) -> Option<ZoomRange> {
        (min <= max && max <= MAX_TILE_ZOOM).then_some(ZoomRange { min, max })
    }

    pub fn min(self) -> u8 {
        self.min
    }

    pub fn max(self) -> u8 {
        self.max
    }
}

/// The tile zoom for a scale (metres per pixel) at a latitude, ADR §3:
/// `floor(log2(156543.03 * cos(lat) / scale))`, so that one pixel is about
/// one 256-pixel tile pixel, then clamped to `range`. Above `range.max` the
/// tiles of `range.max` are enlarged (overzoom).
pub fn tile_zoom(scale: f64, lat: f64, range: ZoomRange) -> u8 {
    let lat = if lat.is_finite() { lat } else { 0.0 };
    zoom_for_world_per_pixel(clamp_scale(scale) / metres_per_world_unit(lat), range)
}

fn zoom_for_world_per_pixel(wpp: f64, range: ZoomRange) -> u8 {
    // A tiny epsilon so that a scale of exactly 156543.03/2^z gives z.
    let ideal = (1.0 / (TILE_PIXELS * wpp)).log2() + 1e-9;
    let ideal = if ideal.is_finite() {
        ideal.floor()
    } else {
        0.0
    };
    let z = ideal.clamp(0.0, f64::from(MAX_TILE_ZOOM)) as u8;
    z.clamp(range.min, range.max)
}

// ---------------------------------------------------------------------------
// View, zoom ladder, viewport
// ---------------------------------------------------------------------------

/// A scale in metres per pixel within [`MIN_SCALE`, `MAX_SCALE`]; NaN,
/// infinite, zero or negative values give [`MAX_SCALE`] (the whole world).
pub fn clamp_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        MAX_SCALE
    }
}

/// The next finer step of [`ZOOM_LADDER`] (stays at the finest). A scale
/// between two steps goes to the finer one.
pub fn step_in(scale: f64) -> f64 {
    let s = clamp_scale(scale);
    ZOOM_LADDER
        .iter()
        .rev()
        .copied()
        .find(|&step| step < s * (1.0 - 1e-9))
        .unwrap_or(MIN_SCALE)
}

/// The next coarser step of [`ZOOM_LADDER`] (stays at the coarsest).
pub fn step_out(scale: f64) -> f64 {
    let s = clamp_scale(scale);
    ZOOM_LADDER
        .iter()
        .copied()
        .find(|&step| step > s * (1.0 + 1e-9))
        .unwrap_or(MAX_SCALE)
}

/// What the map shows: a centre and a scale. The centre's latitude stays
/// within ±[`VIEW_MAX_LAT`]; the scale within the ladder's range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    center: LatLon,
    scale: f64,
}

impl View {
    pub fn new(center: LatLon, scale: f64) -> View {
        View {
            center: LatLon::wrapped(center.lat.clamp(-VIEW_MAX_LAT, VIEW_MAX_LAT), center.lon),
            scale: clamp_scale(scale),
        }
    }

    pub fn center(&self) -> LatLon {
        self.center
    }

    /// Metres per pixel at the centre.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn set_center(&mut self, center: LatLon) {
        *self = View::new(center, self.scale);
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.scale = clamp_scale(scale);
    }

    pub fn zoom_in(&mut self) {
        self.scale = step_in(self.scale);
    }

    pub fn zoom_out(&mut self) {
        self.scale = step_out(self.scale);
    }

    /// World units per pixel at this view's centre.
    pub fn world_per_pixel(&self) -> f64 {
        self.scale / metres_per_world_unit(self.center.lat)
    }

    /// Move the centre by `dx`, `dy` pixels (east and south positive). The
    /// scale in metres per pixel is kept. Non-finite moves are ignored.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        let wpp = self.world_per_pixel();
        let c = project(self.center);
        let dx = if dx.is_finite() { dx } else { 0.0 };
        let dy = if dy.is_finite() { dy } else { 0.0 };
        let moved = unproject(World {
            x: c.x + dx * wpp,
            y: c.y + dy * wpp,
        });
        self.set_center(moved);
    }

    /// This view drawn on `width` x `height` pixels (each clamped to
    /// 1..=[`MAX_VIEW_PIXELS`]).
    pub fn viewport(&self, width: u32, height: u32) -> Viewport {
        Viewport {
            center: project(self.center),
            wpp: self.world_per_pixel(),
            width: width.clamp(1, MAX_VIEW_PIXELS),
            height: height.clamp(1, MAX_VIEW_PIXELS),
        }
    }
}

/// The view asked for more than [`MAX_VIEW_TILES`] tiles at that zoom
/// (a pack whose minimum zoom is too detailed for this scale): draw nothing
/// from that pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyTiles;

/// One tile placed in a view. `wrap` counts whole worlds east (+) or west
/// (-) of the main copy: near the antimeridian, or at coarse scales, the
/// same tile can appear several times with different `wrap`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewTile {
    pub id: TileId,
    pub wrap: i32,
}

/// Tile-local coordinates (`0..extent`, possibly outside for the buffer)
/// to screen pixels: `px = offset_x + x * scale`, `py = offset_y + y *
/// scale`. One multiply-add per axis and vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileTransform {
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

impl TileTransform {
    /// Continuous screen position (pixel `(i, j)` covers `[i, i+1) x [j,
    /// j+1)`).
    #[inline]
    pub fn apply(&self, x: i32, y: i32) -> (f64, f64) {
        (
            self.offset_x + f64::from(x) * self.scale,
            self.offset_y + f64::from(y) * self.scale,
        )
    }

    /// The pixel holding the vertex. Far-away vertices saturate at the
    /// `i32` range instead of wrapping (the renderer clips them).
    #[inline]
    pub fn apply_pixel(&self, x: i32, y: i32) -> (i32, i32) {
        let (px, py) = self.apply(x, y);
        (px.floor() as i32, py.floor() as i32)
    }
}

/// A [`View`] at a size in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    center: World,
    wpp: f64,
    width: u32,
    height: u32,
}

impl Viewport {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// World units per pixel (the same across the whole view: Mercator is
    /// drawn as a flat square).
    pub fn world_per_pixel(&self) -> f64 {
        self.wpp
    }

    /// The tile zoom to read for this view from a pack holding `range`.
    pub fn tile_zoom(&self, range: ZoomRange) -> u8 {
        zoom_for_world_per_pixel(self.wpp, range)
    }

    fn left(&self) -> f64 {
        self.center.x - f64::from(self.width) / 2.0 * self.wpp
    }

    fn top(&self) -> f64 {
        self.center.y - f64::from(self.height) / 2.0 * self.wpp
    }

    /// The tiles of zoom `zoom` (clamped to [`MAX_TILE_ZOOM`]) that cover
    /// the view, row by row from the top left. Rows beyond the poles' limit
    /// are left out; columns wrap around the world. The count is computed
    /// before anything is allocated, and more than [`MAX_VIEW_TILES`] is
    /// refused. At the zoom [`Viewport::tile_zoom`] chooses (tiles of at
    /// least 256 pixels, or the whole world at the coarsest scales) the
    /// bound is never reached.
    pub fn tiles(&self, zoom: u8) -> Result<Vec<ViewTile>, TooManyTiles> {
        let z = zoom.min(MAX_TILE_ZOOM);
        let n_tiles = i64::from(tiles_across(z));
        let n = n_tiles as f64;
        let right = self.left() + f64::from(self.width) * self.wpp;
        let bottom = self.top() + f64::from(self.height) * self.wpp;
        // All finite: centre in the square, wpp bounded, sizes bounded.
        let tx0 = (self.left() * n).floor() as i64;
        let tx1 = (right * n).ceil() as i64 - 1;
        let ty0 = ((self.top() * n).floor() as i64).max(0);
        let ty1 = ((bottom * n).ceil() as i64 - 1).min(n_tiles - 1);
        if tx1 < tx0 || ty1 < ty0 {
            return Ok(Vec::new());
        }
        let count = (tx1 - tx0 + 1)
            .checked_mul(ty1 - ty0 + 1)
            .ok_or(TooManyTiles)?;
        if count > MAX_VIEW_TILES as i64 {
            return Err(TooManyTiles);
        }
        let mut tiles = Vec::with_capacity(count as usize);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let x = tx.rem_euclid(n_tiles) as u32;
                let wrap = i32::try_from(tx.div_euclid(n_tiles)).map_err(|_| TooManyTiles)?;
                let id = TileId { z, x, y: ty as u32 };
                tiles.push(ViewTile { id, wrap });
            }
        }
        Ok(tiles)
    }

    /// The transform from a tile's own coordinates (`0..extent`) to this
    /// view's pixels. Overzoom needs nothing special: a tile of a lower
    /// zoom simply gets a larger `scale`.
    pub fn tile_transform(&self, tile: ViewTile, extent: u32) -> TileTransform {
        let n = f64::from(tiles_across(tile.id.z));
        let extent = f64::from(extent.max(1));
        let column = f64::from(tile.id.x) + f64::from(tile.wrap) * n;
        TileTransform {
            scale: 1.0 / (extent * n * self.wpp),
            offset_x: (column / n - self.left()) / self.wpp,
            offset_y: (f64::from(tile.id.y) / n - self.top()) / self.wpp,
        }
    }

    /// Where `p` is drawn: the copy of the world nearest the centre.
    /// Continuous pixel coordinates; the centre is `(width/2, height/2)`.
    pub fn to_pixel(&self, p: LatLon) -> (f64, f64) {
        let w = project(p);
        let mut dx = w.x - self.center.x;
        dx -= dx.round();
        (
            f64::from(self.width) / 2.0 + dx / self.wpp,
            f64::from(self.height) / 2.0 + (w.y - self.center.y) / self.wpp,
        )
    }

    /// The position under continuous pixel coordinates `(x, y)`; non-finite
    /// coordinates give the centre.
    pub fn from_pixel(&self, x: f64, y: f64) -> LatLon {
        let x = if x.is_finite() {
            x
        } else {
            f64::from(self.width) / 2.0
        };
        let y = if y.is_finite() {
            y
        } else {
            f64::from(self.height) / 2.0
        };
        unproject(World {
            x: self.left() + x * self.wpp,
            y: self.top() + y * self.wpp,
        })
    }
}

// ---------------------------------------------------------------------------
// Distance and direction
// ---------------------------------------------------------------------------

/// Great-circle distance in metres on a sphere of [`EARTH_MEAN_RADIUS_M`]
/// (haversine; within about 0.5% of the ellipsoid).
pub fn distance_m(a: LatLon, b: LatLon) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dp = p2 - p1;
    let dl = (b.lon - a.lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_MEAN_RADIUS_M * h.clamp(0.0, 1.0).sqrt().asin()
}

/// Initial bearing from `a` towards `b` in degrees clockwise from north,
/// in [0, 360). `None` when the points are less than half a metre apart
/// (there is no direction).
pub fn initial_bearing(a: LatLon, b: LatLon) -> Option<f64> {
    if distance_m(a, b) < 0.5 {
        return None;
    }
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    let deg = y.atan2(x).to_degrees().rem_euclid(360.0);
    Some(if deg >= 360.0 { 0.0 } else { deg })
}

/// The eight compass directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compass {
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

impl Compass {
    const ALL: [Compass; 8] = [
        Compass::N,
        Compass::NE,
        Compass::E,
        Compass::SE,
        Compass::S,
        Compass::SW,
        Compass::W,
        Compass::NW,
    ];

    /// The sector of 45° around `degrees` (any finite value; 22.5 belongs
    /// to NE). `None` for NaN or infinities.
    pub fn from_bearing(degrees: f64) -> Option<Compass> {
        if !degrees.is_finite() {
            return None;
        }
        let sector = ((degrees.rem_euclid(360.0) + 22.5) / 45.0).floor() as usize % 8;
        Some(Compass::ALL[sector])
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Compass::N => "N",
            Compass::NE => "NE",
            Compass::E => "E",
            Compass::SE => "SE",
            Compass::S => "S",
            Compass::SW => "SW",
            Compass::W => "W",
            Compass::NW => "NW",
        }
    }
}

impl fmt::Display for Compass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A distance for people: `"42 m"` (to the metre below 100 m), `"350 m"`
/// (to 10 m), `"1.5 km"` (to 100 m below 10 km), `"15 km"`. A negative or
/// non-finite value gives `"unknown"`.
pub fn format_distance(metres: f64) -> String {
    if !metres.is_finite() || metres < 0.0 {
        return "unknown".to_string();
    }
    if metres < 99.5 {
        return format!("{} m", metres.round() as u64);
    }
    let tens = (metres / 10.0).round() as u64 * 10;
    if tens < 1000 {
        return format!("{tens} m");
    }
    // Each unit is at least the previous one's limit, so a value that
    // rounded up into this branch never prints below it (995 m: 1.0 km).
    let tenths_km = ((metres / 100.0).round() as u64).max(10);
    if tenths_km < 100 {
        return format!("{}.{} km", tenths_km / 10, tenths_km % 10);
    }
    format!("{} km", ((metres / 1000.0).round() as u64).max(10))
}

/// Distance and direction from `from` to `to`, as in search results:
/// `"15 km SW"`, or `"here"` under half a metre.
pub fn describe_offset(from: LatLon, to: LatLon) -> String {
    match initial_bearing(from, to).and_then(Compass::from_bearing) {
        Some(dir) => format!("{} {dir}", format_distance(distance_m(from, to))),
        None => "here".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Coordinate formatting
// ---------------------------------------------------------------------------

fn hemisphere(value: f64, rounded_is_zero: bool, pos: char, neg: char) -> char {
    if value < 0.0 && !rounded_is_zero {
        neg
    } else {
        pos
    }
}

fn decimal_part(value: f64, pos: char, neg: char) -> String {
    let units = (value.abs() * 1e5).round() as u64;
    let letter = hemisphere(value, units == 0, pos, neg);
    format!("{}.{:05} {letter}", units / 100_000, units % 100_000)
}

/// Decimal degrees with 5 decimals (about a metre): `49.61160 N  6.13190 E`.
pub fn format_decimal(p: LatLon) -> String {
    format!(
        "{}  {}",
        decimal_part(p.lat, 'N', 'S'),
        decimal_part(p.lon, 'E', 'W')
    )
}

fn dms_part(value: f64, pos: char, neg: char) -> String {
    // Whole tenths of an arcsecond, so 59.96" carries into the minute.
    let tenths = (value.abs() * 36_000.0).round() as u64;
    let letter = hemisphere(value, tenths == 0, pos, neg);
    let (deg, rest) = (tenths / 36_000, tenths % 36_000);
    let (min, sec) = (rest / 600, rest % 600);
    format!("{deg}d{min:02}'{:02}.{}\" {letter}", sec / 10, sec % 10)
}

/// Degrees, minutes and seconds to a tenth of a second (about 3 m), in
/// ASCII because `°` is not in the console font's glyph set (CLAUDE.md §4):
/// `49d36'41.8" N  6d07'54.8" E`. [`parse_coordinates`] reads it back.
pub fn format_dms(p: LatLon) -> String {
    format!(
        "{}  {}",
        dms_part(p.lat, 'N', 'S'),
        dms_part(p.lon, 'E', 'W')
    )
}

// ---------------------------------------------------------------------------
// Coordinate parsing ("go to coordinates")
// ---------------------------------------------------------------------------

/// Why typed coordinates were refused. `Display` gives a sentence for a
/// beginner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordError {
    Empty,
    TooLong,
    /// A character that has no meaning in coordinates.
    UnexpectedCharacter(char),
    /// Something that looks like a number but is not one (`1.2.3`, `-`).
    BadNumber,
    /// Not recognisable as one latitude and one longitude.
    Layout,
    /// Letters must be given for both coordinates (N or S, then E or W) or
    /// for neither, once each.
    Letters,
    /// A minus sign and a letter on the same coordinate.
    SignAndLetter,
    /// Degrees, minutes and seconds in a wrong form: more than three parts,
    /// marks out of order, a sign or decimals before the last part.
    BadDms,
    MinutesTooLarge,
    SecondsTooLarge,
    LatitudeOutOfRange,
    LongitudeOutOfRange,
}

impl fmt::Display for CoordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoordError::Empty => f.write_str(
                "type a latitude and a longitude, for example 49.6116 6.1319 or 49.6116 N 6.1319 E",
            ),
            CoordError::TooLong => f.write_str("that is too long for coordinates"),
            CoordError::UnexpectedCharacter(c) if c.is_ascii_graphic() => {
                write!(f, "the character {c} cannot be used in coordinates")
            }
            CoordError::UnexpectedCharacter(_) => {
                f.write_str("there is a character that cannot be used in coordinates")
            }
            CoordError::BadNumber => f.write_str("one of the numbers is not written correctly"),
            CoordError::Layout => {
                f.write_str("write the latitude, then the longitude, for example 49.6116, 6.1319")
            }
            CoordError::Letters => f.write_str(
                "give N or S for the latitude and E or W for the longitude, or no letters at all",
            ),
            CoordError::SignAndLetter => {
                f.write_str("use either a minus sign or a letter (S or W), not both")
            }
            CoordError::BadDms => f.write_str(
                "write degrees, minutes and seconds in that order, for example 49 36 41.8 N",
            ),
            CoordError::MinutesTooLarge => f.write_str("minutes must be less than 60"),
            CoordError::SecondsTooLarge => f.write_str("seconds must be less than 60"),
            CoordError::LatitudeOutOfRange => {
                f.write_str("the latitude must be between 90 S and 90 N")
            }
            CoordError::LongitudeOutOfRange => {
                f.write_str("the longitude must be between 180 W and 180 E")
            }
        }
    }
}

impl std::error::Error for CoordError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Deg,
    Min,
    Sec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hemi {
    N,
    S,
    E,
    W,
}

#[derive(Debug, Clone, Copy)]
struct Num {
    value: f64,
    negative: bool,
    signed: bool,
    fraction: bool,
    unit: Option<Unit>,
}

#[derive(Debug, Clone, Copy)]
enum Tok {
    Num(Num),
    Hemi(Hemi),
    /// `,`, `;` or `/` between the two coordinates.
    Sep,
}

/// Longest number accepted, in characters (sign, digits and point).
const MAX_NUMBER_CHARS: usize = 16;

fn is_minute_mark(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{2032}')
}

fn tokenize(chars: &[char], decimal_comma: bool) -> Result<Vec<Tok>, CoordError> {
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mark = match c {
            '\u{b0}' | '\u{ba}' | 'd' | 'D' => Some(Unit::Deg),
            c if is_minute_mark(c) => {
                if chars.get(i + 1).copied().is_some_and(is_minute_mark) {
                    i += 1;
                    Some(Unit::Sec)
                } else {
                    Some(Unit::Min)
                }
            }
            '"' | '\u{201d}' | '\u{2033}' => Some(Unit::Sec),
            _ => None,
        };
        if let Some(unit) = mark {
            match toks.last_mut() {
                Some(Tok::Num(n)) if n.unit.is_none() => n.unit = Some(unit),
                _ => return Err(CoordError::BadDms),
            }
            i += 1;
            continue;
        }
        let hemi = match c {
            'N' | 'n' => Some(Hemi::N),
            'S' | 's' => Some(Hemi::S),
            'E' | 'e' => Some(Hemi::E),
            'W' | 'w' => Some(Hemi::W),
            _ => None,
        };
        if let Some(h) = hemi {
            toks.push(Tok::Hemi(h));
            i += 1;
            continue;
        }
        match c {
            c if c.is_whitespace() => i += 1,
            ',' | ';' | '/' if !(c == ',' && decimal_comma) => {
                toks.push(Tok::Sep);
                i += 1;
            }
            '0'..='9' | '.' | ',' | '+' | '-' | '\u{2212}' => {
                let (num, next) = scan_number(chars, i, decimal_comma)?;
                toks.push(Tok::Num(num));
                i = next;
            }
            other => return Err(CoordError::UnexpectedCharacter(other)),
        }
    }
    Ok(toks)
}

/// A number starting at `chars[start]`: optional sign, digits, at most one
/// decimal point (or comma when `decimal_comma`).
fn scan_number(
    chars: &[char],
    start: usize,
    decimal_comma: bool,
) -> Result<(Num, usize), CoordError> {
    let mut i = start;
    let mut text = String::new();
    let (mut negative, mut signed) = (false, false);
    if let Some(&c) = chars.get(i)
        && matches!(c, '+' | '-' | '\u{2212}')
    {
        signed = true;
        negative = c != '+';
        i += 1;
    }
    let (mut digits, mut fraction) = (0usize, false);
    while let Some(&c) = chars.get(i) {
        if c.is_ascii_digit() {
            digits += 1;
            text.push(c);
        } else if c == '.' || (c == ',' && decimal_comma) {
            if fraction {
                return Err(CoordError::BadNumber);
            }
            fraction = true;
            text.push('.');
        } else {
            break;
        }
        if text.len() > MAX_NUMBER_CHARS {
            return Err(CoordError::BadNumber);
        }
        i += 1;
    }
    if digits == 0 {
        return Err(CoordError::BadNumber);
    }
    let value: f64 = text.parse().map_err(|_| CoordError::BadNumber)?;
    if !value.is_finite() {
        return Err(CoordError::BadNumber);
    }
    let num = Num {
        value,
        negative,
        signed,
        fraction,
        unit: None,
    };
    Ok((num, i))
}

/// A comma is a decimal comma only when the input has no point, contains a
/// space or semicolon to separate the coordinates, and every comma sits
/// between two digits (`49,6116 6,1319`). Otherwise commas separate.
fn uses_decimal_comma(chars: &[char]) -> bool {
    if chars.contains(&'.') || !chars.iter().any(|&c| c.is_whitespace() || c == ';') {
        return false;
    }
    let mut any = false;
    for (i, &c) in chars.iter().enumerate() {
        if c == ',' {
            any = true;
            let before = i
                .checked_sub(1)
                .and_then(|j| chars.get(j))
                .is_some_and(char::is_ascii_digit);
            let after = chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if !(before && after) {
                return false;
            }
        }
    }
    any
}

fn only_numbers(toks: &[Tok]) -> Result<Vec<Num>, CoordError> {
    toks.iter()
        .map(|t| match t {
            Tok::Num(n) => Ok(*n),
            _ => Err(CoordError::Layout),
        })
        .collect()
}

type Component = (Vec<Num>, Option<Hemi>);

fn split_components(toks: &[Tok]) -> Result<(Component, Component), CoordError> {
    let hemis: Vec<usize> = toks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| matches!(t, Tok::Hemi(_)).then_some(i))
        .collect();
    let hemi_at = |i: usize| match toks.get(i) {
        Some(Tok::Hemi(h)) => Some(*h),
        _ => None,
    };
    match hemis.as_slice() {
        [] => {
            let seps: Vec<usize> = toks
                .iter()
                .enumerate()
                .filter_map(|(i, t)| matches!(t, Tok::Sep).then_some(i))
                .collect();
            match seps.as_slice() {
                [s] => Ok((
                    (only_numbers(&toks[..*s])?, None),
                    (only_numbers(&toks[s + 1..])?, None),
                )),
                [] => {
                    let nums = only_numbers(toks)?;
                    let degrees: Vec<usize> = nums
                        .iter()
                        .enumerate()
                        .filter_map(|(i, n)| (n.unit == Some(Unit::Deg)).then_some(i))
                        .collect();
                    let at = match degrees.as_slice() {
                        [0, second] => *second,
                        _ if nums.len() % 2 == 0 && nums.len() <= 6 => nums.len() / 2,
                        _ => return Err(CoordError::Layout),
                    };
                    let (a, b) = nums.split_at(at);
                    Ok(((a.to_vec(), None), (b.to_vec(), None)))
                }
                _ => Err(CoordError::Layout),
            }
        }
        [a, b] => {
            let (a, b) = (*a, *b);
            let (first, second) = if a == 0 {
                // N 49.6 E 6.1
                let mut first = &toks[1..b];
                if let [rest @ .., Tok::Sep] = first {
                    first = rest;
                }
                ((first, hemi_at(0)), (&toks[b + 1..], hemi_at(b)))
            } else if b == toks.len() - 1 {
                // 49.6 N 6.1 E
                let mut second = &toks[a + 1..b];
                if let [Tok::Sep, rest @ ..] = second {
                    second = rest;
                }
                ((&toks[..a], hemi_at(a)), (second, hemi_at(b)))
            } else {
                return Err(CoordError::Layout);
            };
            Ok((
                (only_numbers(first.0)?, first.1),
                (only_numbers(second.0)?, second.1),
            ))
        }
        _ => Err(CoordError::Letters),
    }
}

/// Signed degrees of one coordinate.
fn component_value(nums: &[Num], hemi: Option<Hemi>) -> Result<f64, CoordError> {
    const UNITS: [Unit; 3] = [Unit::Deg, Unit::Min, Unit::Sec];
    if nums.is_empty() {
        return Err(CoordError::Layout);
    }
    if nums.len() > UNITS.len() {
        return Err(CoordError::BadDms);
    }
    let last = nums.len() - 1;
    for (i, n) in nums.iter().enumerate() {
        if n.unit.is_some_and(|u| u != UNITS[i]) || (i > 0 && n.signed) || (i < last && n.fraction)
        {
            return Err(CoordError::BadDms);
        }
    }
    if nums[0].signed && hemi.is_some() {
        return Err(CoordError::SignAndLetter);
    }
    let minutes = nums.get(1).map_or(0.0, |n| n.value);
    let seconds = nums.get(2).map_or(0.0, |n| n.value);
    if minutes >= 60.0 {
        return Err(CoordError::MinutesTooLarge);
    }
    if seconds >= 60.0 {
        return Err(CoordError::SecondsTooLarge);
    }
    let magnitude = nums[0].value + minutes / 60.0 + seconds / 3600.0;
    let negative = nums[0].negative || matches!(hemi, Some(Hemi::S | Hemi::W));
    Ok(if negative { -magnitude } else { magnitude })
}

/// Read coordinates typed by a person, for "go to coordinates".
///
/// Accepted, in any mix the grammar allows:
/// - decimal degrees: `49.6116 6.1319`, `49.6116, 6.1319`, `-33.87;151.21`,
///   `49,6116 6,1319` (decimal commas, see below);
/// - degrees, minutes, seconds: `49 36 41.8 N 6 7 54.8 E`,
///   `49°36'41.8"N 6°7'54.8"E`, `49d36'41.8" N  6d07'54.8" E` (what
///   [`format_dms`] prints), `49° 36.7' N, 6° 7.9' E`;
/// - hemisphere letters N/S/E/W (either case) before or after each
///   coordinate, in which case the order may be swapped (`6.1 E 49.6 N`);
///   without letters the latitude comes first, and a minus sign means
///   south or west.
///
/// Marks: `°` `º` `d` for degrees, `'` `’` `′` for minutes, `"` `”` `″` or
/// two quotes for seconds. Without letters the two coordinates are split
/// at a `,` `;` or `/`, else before the second degree mark, else in the
/// middle (2, 4 or 6 numbers). Ranges are strict: |latitude| <= 90,
/// |longitude| <= 180, minutes and seconds below 60.
///
/// Work is bounded: input over [`MAX_COORD_INPUT`] characters is refused
/// before anything else.
pub fn parse_coordinates(input: &str) -> Result<LatLon, CoordError> {
    if input.chars().take(MAX_COORD_INPUT + 1).count() > MAX_COORD_INPUT {
        return Err(CoordError::TooLong);
    }
    let chars: Vec<char> = input.trim().chars().collect();
    if chars.is_empty() {
        return Err(CoordError::Empty);
    }
    let toks = tokenize(&chars, uses_decimal_comma(&chars))?;
    let ((nums1, hemi1), (nums2, hemi2)) = split_components(&toks)?;
    if nums1.is_empty() || nums2.is_empty() {
        return Err(CoordError::Layout);
    }
    let v1 = component_value(&nums1, hemi1)?;
    let v2 = component_value(&nums2, hemi2)?;
    let is_lat = |h: Option<Hemi>| matches!(h, Some(Hemi::N | Hemi::S));
    let (lat, lon) = match (hemi1, hemi2) {
        (None, None) => (v1, v2),
        (Some(_), Some(_)) if is_lat(hemi1) && !is_lat(hemi2) => (v1, v2),
        (Some(_), Some(_)) if !is_lat(hemi1) && is_lat(hemi2) => (v2, v1),
        _ => return Err(CoordError::Letters),
    };
    if lat.abs() > 90.0 {
        return Err(CoordError::LatitudeOutOfRange);
    }
    if lon.abs() > 180.0 {
        return Err(CoordError::LongitudeOutOfRange);
    }
    LatLon::new(lat, lon).ok_or(CoordError::Layout)
}

// ---------------------------------------------------------------------------
// Scale bar
// ---------------------------------------------------------------------------

/// A scale bar: `cells` cells long (one cell is one pixel wide), standing
/// for `metres`, labelled `label` (`"500 m"`, `"2 km"`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleBar {
    pub cells: u32,
    pub metres: u64,
    pub label: String,
}

/// The longest 1-2-5 length (1 m to 50,000 km) whose bar, at `scale`
/// metres per pixel, is between `min_cells` and `max_cells` cells after
/// rounding. Since the scale is the one at the centre, the bar is true at
/// the centre's latitude. `None` for a non-finite or non-positive scale, a
/// width range that is empty or starts at 0, or when no length fits.
pub fn scale_bar(scale: f64, min_cells: u32, max_cells: u32) -> Option<ScaleBar> {
    if !scale.is_finite() || scale <= 0.0 || min_cells == 0 || min_cells > max_cells {
        return None;
    }
    let mut best = None;
    let mut decade: u64 = 1;
    for _ in 0..8 {
        for mantissa in [1, 2, 5] {
            let metres = mantissa * decade;
            let cells = (metres as f64 / scale).round();
            if cells >= f64::from(min_cells) && cells <= f64::from(max_cells) {
                best = Some((cells as u32, metres));
            }
        }
        decade *= 10;
    }
    best.map(|(cells, metres)| ScaleBar {
        cells,
        metres,
        label: if metres < 1000 {
            format!("{metres} m")
        } else {
            format!("{} km", metres / 1000)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ll(lat: f64, lon: f64) -> LatLon {
        LatLon::new(lat, lon).expect("valid test position")
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    const LUXEMBOURG: (f64, f64) = (49.6116, 6.1319);
    const ESCH: (f64, f64) = (49.4958, 5.9806);
    const PARIS: (f64, f64) = (48.8566, 2.3522);
    const LONDON: (f64, f64) = (51.5074, -0.1278);
    const NEW_YORK: (f64, f64) = (40.7128, -74.0060);
    const SYDNEY: (f64, f64) = (-33.8688, 151.2093);

    fn at(p: (f64, f64)) -> LatLon {
        ll(p.0, p.1)
    }

    #[test]
    fn positions_validate_and_wrap() {
        assert!(LatLon::new(91.0, 0.0).is_none());
        assert!(LatLon::new(0.0, 180.1).is_none());
        assert!(LatLon::new(f64::NAN, 0.0).is_none());
        assert!(LatLon::new(0.0, f64::INFINITY).is_none());
        assert_eq!(ll(0.0, 180.0).lon(), -180.0);
        assert_eq!(ll(90.0, -180.0).lat(), 90.0);
        assert_eq!(LatLon::wrapped(0.0, 190.0).lon(), -170.0);
        assert_eq!(LatLon::wrapped(0.0, 540.0).lon(), -180.0);
        assert_eq!(LatLon::wrapped(0.0, -540.0).lon(), -180.0);
        assert_eq!(LatLon::wrapped(0.0, -1e-300 - 180.0).lon(), -180.0);
        assert!(close(LatLon::wrapped(0.0, 359.5).lon(), -0.5, 1e-12));
        assert_eq!(LatLon::wrapped(f64::NAN, f64::NEG_INFINITY), ll(0.0, 0.0));
        assert_eq!(LatLon::wrapped(100.0, 0.0).lat(), 90.0);
        assert_eq!(LatLon::wrapped(f64::MAX, f64::MAX).lat(), 90.0);
        assert!(LatLon::wrapped(0.0, 1e300).lon().is_finite());
    }

    #[test]
    fn projection_round_trips() {
        for p in [
            LUXEMBOURG,
            SYDNEY,
            NEW_YORK,
            (0.0, 0.0),
            (0.0, -180.0),
            (-12.5, 179.999_999),
            (85.0, 30.0),
            (-85.0, -30.0),
        ] {
            let back = unproject(project(at(p)));
            assert!(close(back.lat(), p.0, 1e-9), "{p:?} -> {back:?}");
            assert!(close(back.lon(), p.1, 1e-9), "{p:?} -> {back:?}");
        }
        let origin = project(ll(0.0, 0.0));
        assert!(close(origin.x, 0.5, 1e-15) && close(origin.y, 0.5, 1e-15));
    }

    #[test]
    fn projection_clamps_at_the_poles() {
        assert_eq!(project(ll(90.0, 0.0)).y, 0.0);
        assert_eq!(project(ll(-90.0, 0.0)).y, 1.0);
        assert!(close(project(ll(MERCATOR_MAX_LAT, 0.0)).y, 0.0, 1e-12));
        assert!(close(
            unproject(World { x: 0.5, y: 0.0 }).lat(),
            MERCATOR_MAX_LAT,
            1e-9
        ));
        assert!(close(
            unproject(World { x: 0.5, y: -3.0 }).lat(),
            MERCATOR_MAX_LAT,
            1e-9
        ));
        assert!(close(
            unproject(World { x: 0.5, y: 7.0 }).lat(),
            -MERCATOR_MAX_LAT,
            1e-9
        ));
        let mid = unproject(World {
            x: f64::NAN,
            y: f64::INFINITY,
        });
        assert_eq!((mid.lat(), mid.lon()), (0.0, 0.0));
        // x wraps around the world.
        assert!(close(
            unproject(World { x: 1.25, y: 0.5 }).lon(),
            -90.0,
            1e-9
        ));
        assert!(close(
            unproject(World { x: -0.25, y: 0.5 }).lon(),
            90.0,
            1e-9
        ));
        assert_eq!(unproject(World { x: 1.0, y: 0.5 }).lon(), -180.0);
    }

    #[test]
    fn tile_numbers_of_known_places() {
        // The OpenStreetMap wiki's "Slippy map tilenames" example.
        let gate = ll(52.516_280_112_623_04, 13.377_714_963_619_61);
        assert_eq!(
            TileId::containing(gate, 17),
            TileId::new(17, 70406, 42987).expect("valid")
        );
        let lux = at(LUXEMBOURG);
        assert_eq!(
            TileId::containing(lux, 14),
            TileId::new(14, 8471, 5583).expect("valid")
        );
        assert_eq!(
            TileId::containing(lux, 10),
            TileId::new(10, 529, 348).expect("valid")
        );
        assert_eq!(
            TileId::containing(at(PARIS), 10),
            TileId::new(10, 518, 352).expect("valid")
        );
        assert_eq!(
            TileId::containing(at(SYDNEY), 8),
            TileId::new(8, 235, 153).expect("valid")
        );
        assert_eq!(
            TileId::containing(at(SYDNEY), 0),
            TileId::new(0, 0, 0).expect("valid")
        );
        // Edges: the poles land in the first and last rows, 180 wraps.
        assert_eq!(TileId::containing(ll(90.0, 0.0), 3).y(), 0);
        assert_eq!(TileId::containing(ll(-90.0, 0.0), 3).y(), 7);
        assert_eq!(TileId::containing(ll(0.0, 180.0), 3).x(), 0);
        assert_eq!(TileId::containing(ll(0.0, 179.999_999), 3).x(), 7);
        assert_eq!(TileId::containing(lux, 99).z(), MAX_TILE_ZOOM);
    }

    #[test]
    fn tms_rows_are_flipped_and_validated() {
        let xyz = TileId::new(14, 8471, 5583).expect("valid");
        let tms = xyz.to_tms();
        assert_eq!(
            (tms.zoom(), tms.column(), tms.row()),
            (14, 8471, 16383 - 5583)
        );
        assert_eq!(tms.to_xyz(), xyz);
        assert_eq!(TileId::new(0, 0, 0).expect("valid").to_tms().row(), 0);
        let top = TileId::new(2, 1, 0).expect("valid").to_tms();
        assert_eq!(top.row(), 3);
        assert!(TileId::new(2, 4, 0).is_none());
        assert!(TileId::new(2, 0, 4).is_none());
        assert!(TileId::new(MAX_TILE_ZOOM + 1, 0, 0).is_none());
        assert!(TmsTile::new(3, 8, 0).is_none());
        assert!(TmsTile::new(3, 0, 8).is_none());
        assert!(TmsTile::new(MAX_TILE_ZOOM, (1 << 24) - 1, (1 << 24) - 1).is_some());
        assert!(ZoomRange::new(5, 4).is_none());
        assert!(ZoomRange::new(0, MAX_TILE_ZOOM + 1).is_none());
    }

    #[test]
    fn zoom_ladder_steps_in_and_out() {
        for pair in ZOOM_LADDER.windows(2) {
            assert_eq!(step_in(pair[1]), pair[0]);
            assert_eq!(step_out(pair[0]), pair[1]);
        }
        assert_eq!(step_in(MIN_SCALE), MIN_SCALE);
        assert_eq!(step_out(MAX_SCALE), MAX_SCALE);
        assert_eq!(step_in(30.0), 20.0);
        assert_eq!(step_out(30.0), 50.0);
        assert_eq!(step_in(1.0), MIN_SCALE);
        assert_eq!(step_out(1e9), MAX_SCALE);
        assert_eq!(clamp_scale(f64::NAN), MAX_SCALE);
        assert_eq!(clamp_scale(-3.0), MAX_SCALE);
        assert_eq!(clamp_scale(0.0), MAX_SCALE);
        assert_eq!(step_out(f64::INFINITY), MAX_SCALE);
        let mut view = View::new(at(LUXEMBOURG), 1_000.0);
        view.zoom_in();
        assert_eq!(view.scale(), 500.0);
        for _ in 0..40 {
            view.zoom_in();
        }
        assert_eq!(view.scale(), MIN_SCALE);
        for _ in 0..40 {
            view.zoom_out();
        }
        assert_eq!(view.scale(), MAX_SCALE);
    }

    #[test]
    fn tile_zoom_for_each_step() {
        let range = ZoomRange::new(0, 14).expect("valid");
        // floor(log2(156543.03 * cos 45° / m)) at each ladder step.
        let expected = [14, 13, 12, 11, 10, 9, 7, 6, 5, 4, 3, 2, 1, 0, 0];
        for (scale, z) in ZOOM_LADDER.iter().zip(expected) {
            assert_eq!(tile_zoom(*scale, 45.0, range), z, "scale {scale}");
            let vp = View::new(ll(45.0, 3.0), *scale).viewport(100, 50);
            assert_eq!(vp.tile_zoom(range), z, "scale {scale}");
        }
        // Exactly one tile pixel per pixel at the equator gives that zoom.
        let wide = ZoomRange::new(0, MAX_TILE_ZOOM).expect("valid");
        for z in 0..=18u8 {
            let scale = EARTH_CIRCUMFERENCE_M / TILE_PIXELS / f64::from(1u32 << z);
            assert_eq!(
                tile_zoom(scale.max(MIN_SCALE), 0.0, wide),
                z.min(14),
                "z {z}"
            );
        }
        // Clamped to the pack's range: overzoom below, coarse floor above.
        let mid = ZoomRange::new(3, 7).expect("valid");
        assert_eq!(tile_zoom(MIN_SCALE, 45.0, mid), 7);
        assert_eq!(tile_zoom(MAX_SCALE, 45.0, mid), 3);
        assert_eq!(tile_zoom(f64::NAN, f64::NAN, range), 0);
        // Far north a tile covers less ground: the same scale needs a lower
        // zoom.
        assert!(tile_zoom(1_000.0, 80.0, range) < tile_zoom(1_000.0, 0.0, range));
    }

    #[test]
    fn tiles_per_view_are_bounded() {
        let range = ZoomRange::new(0, 14).expect("valid");
        let sizes = [
            (1, 1),
            (80, 50),
            (170, 96),
            (480, 270),
            (1024, 1024),
            (5000, 9000),
        ];
        let mut max_screen = 0;
        for &scale in &ZOOM_LADDER {
            for lat in [-85.0, -60.0, -10.0, 0.0, 45.0, 70.0, 85.0] {
                for lon in [-180.0, -179.99, 0.0, 6.1, 179.99] {
                    let view = View::new(ll(lat, lon), scale);
                    for &(w, h) in &sizes {
                        let vp = view.viewport(w, h);
                        assert!(vp.width() <= MAX_VIEW_PIXELS && vp.height() <= MAX_VIEW_PIXELS);
                        let tiles = vp.tiles(vp.tile_zoom(range)).expect("natural zoom fits");
                        assert!(!tiles.is_empty());
                        assert!(tiles.len() <= MAX_VIEW_TILES);
                        if (w, h) == (170, 96) && lat.abs() <= 70.0 {
                            max_screen = max_screen.max(tiles.len());
                        }
                    }
                }
            }
        }
        // ADR §3: a full screen needs 1 to 6 tiles (up to 70° latitude).
        assert!(max_screen <= 6, "{max_screen}");
        // A pack whose minimum zoom is far too detailed for the scale is
        // refused instead of listing thousands of tiles.
        let vp = View::new(ll(0.0, 0.0), MAX_SCALE).viewport(170, 96);
        assert_eq!(vp.tiles(14), Err(TooManyTiles));
        assert_eq!(vp.tiles(200), Err(TooManyTiles));
        let fine = View::new(ll(0.0, 0.0), MIN_SCALE).viewport(1024, 1024);
        assert_eq!(fine.tiles(MAX_TILE_ZOOM), Err(TooManyTiles));
        assert!(fine.tiles(14).is_ok());
    }

    #[test]
    fn tiles_wrap_at_the_antimeridian() {
        let vp = View::new(ll(0.0, 179.999), 2_000.0).viewport(100, 10);
        let z = vp.tile_zoom(ZoomRange::new(0, 14).expect("valid"));
        let tiles = vp.tiles(z).expect("fits");
        let n = 1u32 << z;
        let east = tiles.iter().find(|t| t.id.x() == n - 1 && t.wrap == 0);
        let west = tiles.iter().find(|t| t.id.x() == 0 && t.wrap == 1);
        let (east, west) = (east.expect("east tile"), west.expect("wrapped tile"));
        // The two tiles touch on screen.
        let e = vp.tile_transform(*east, 4096).apply(4096, 0);
        let w = vp.tile_transform(*west, 4096).apply(0, 0);
        assert!(close(e.0, w.0, 1e-6) && close(e.1, w.1, 1e-6));
        // At the coarsest scale the world repeats across a wide screen.
        let world = View::new(ll(60.0, 0.0), MAX_SCALE).viewport(1024, 200);
        let tiles = world.tiles(0).expect("fits");
        assert!(tiles.len() > 2 && tiles.iter().all(|t| t.id == tiles[0].id));
        // Near the northern limit no row above the square is listed.
        let vp = View::new(ll(85.0, 0.0), 1_000.0).viewport(100, 100);
        let tiles = vp.tiles(4).expect("fits");
        assert!(!tiles.is_empty() && tiles.iter().all(|t| t.id.y() == 0));
    }

    #[test]
    fn tile_transform_maps_tile_coordinates_to_pixels() {
        let view = View::new(at(LUXEMBOURG), 50.0);
        let vp = view.viewport(170, 96);
        let range = ZoomRange::new(0, 14).expect("valid");
        let z = vp.tile_zoom(range);
        let centre_tile = TileId::containing(at(LUXEMBOURG), z);
        let tile = ViewTile {
            id: centre_tile,
            wrap: 0,
        };
        assert!(vp.tiles(z).expect("fits").contains(&tile));
        let extent = 4096;
        let t = vp.tile_transform(tile, extent);
        // The centre's tile-local coordinates land on the screen centre.
        let w = project(at(LUXEMBOURG));
        let n = f64::from(1u32 << z);
        let lx = (w.x * n - f64::from(centre_tile.x())) * f64::from(extent);
        let ly = (w.y * n - f64::from(centre_tile.y())) * f64::from(extent);
        let px = t.offset_x + lx * t.scale;
        let py = t.offset_y + ly * t.scale;
        assert!(close(px, 85.0, 1e-6) && close(py, 48.0, 1e-6), "{px} {py}");
        // A tile spans 1 / (2^z * wpp) pixels, at least 256 at natural zoom.
        let span = t.apply(extent as i32, 0).0 - t.apply(0, 0).0;
        assert!(close(span, 1.0 / (n * vp.world_per_pixel()), 1e-6));
        assert!(span >= 256.0);
        // Overzoom: at 5 m per pixel, a pack stopping at z12 has its tiles
        // enlarged to over 1000 pixels.
        let fine = View::new(at(LUXEMBOURG), 5.0).viewport(170, 96);
        assert_eq!(fine.tile_zoom(ZoomRange::new(0, 12).expect("valid")), 12);
        let tile = ViewTile {
            id: TileId::containing(at(LUXEMBOURG), 12),
            wrap: 0,
        };
        let t = fine.tile_transform(tile, 4096);
        let span = t.apply(4096, 0).0 - t.apply(0, 0).0;
        assert!(span > 1000.0, "{span}");
        // Pixels saturate instead of wrapping.
        let huge = TileTransform {
            scale: 1e12,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        assert_eq!(huge.apply_pixel(i32::MAX, i32::MIN), (i32::MAX, i32::MIN));
        let t = vp.tile_transform(tile, 0);
        assert!(t.scale.is_finite());
    }

    #[test]
    fn pixels_and_positions_round_trip() {
        let view = View::new(at(LUXEMBOURG), 20.0);
        let vp = view.viewport(170, 96);
        let (cx, cy) = vp.to_pixel(at(LUXEMBOURG));
        assert!(close(cx, 85.0, 1e-9) && close(cy, 48.0, 1e-9));
        let p = vp.from_pixel(10.5, 70.25);
        let (x, y) = vp.to_pixel(p);
        assert!(close(x, 10.5, 1e-6) && close(y, 70.25, 1e-6));
        let c = vp.from_pixel(f64::NAN, f64::INFINITY);
        assert!(close(c.lat(), LUXEMBOURG.0, 1e-9) && close(c.lon(), LUXEMBOURG.1, 1e-9));
        // 20 m per pixel: a point 1 km east is 50 pixels to the right.
        let east = View::new(ll(0.0, 0.0), 20.0).viewport(170, 96);
        let p = ll(0.0, 1000.0 / EARTH_CIRCUMFERENCE_M * 360.0);
        assert!(close(east.to_pixel(p).0, 85.0 + 50.0, 1e-6));
        // Across the antimeridian the nearest copy is used.
        let anti = View::new(ll(0.0, 179.99), 100.0).viewport(170, 96);
        let (x, _) = anti.to_pixel(ll(0.0, -179.99));
        assert!(x > 85.0 && x < 170.0, "{x}");
    }

    #[test]
    fn panning_wraps_longitude_and_stops_at_85() {
        let mut view = View::new(ll(0.0, 179.9), 1_000.0);
        view.pan_pixels(100.0, 0.0); // about 100 km east, across 180
        assert!(view.center().lon() < -179.0, "{:?}", view.center());
        assert_eq!(view.scale(), 1_000.0);
        let mut north = View::new(ll(84.0, 0.0), 10_000.0);
        for _ in 0..100 {
            north.pan_pixels(0.0, -1000.0);
        }
        assert_eq!(north.center().lat(), VIEW_MAX_LAT);
        let mut south = View::new(ll(-89.0, 0.0), 10_000.0);
        assert_eq!(south.center().lat(), -VIEW_MAX_LAT);
        south.pan_pixels(f64::NAN, f64::INFINITY);
        assert_eq!(south.center().lat(), -VIEW_MAX_LAT);
        let before = View::new(at(LUXEMBOURG), 50.0);
        let mut after = before;
        after.pan_pixels(42.0, -17.0);
        after.pan_pixels(-42.0, 17.0);
        // Not exact: the second move uses the world scale at the new latitude.
        assert!(close(after.center().lat(), before.center().lat(), 1e-4));
        assert!(close(after.center().lon(), before.center().lon(), 1e-4));
    }

    #[test]
    fn distances_between_known_places() {
        let d = distance_m(at(PARIS), at(LONDON));
        assert!(close(d, 343_560.0, 1_000.0), "{d}");
        let d = distance_m(at(NEW_YORK), at(LONDON));
        assert!(close(d, 5_570_000.0, 10_000.0), "{d}");
        let d = distance_m(at(LUXEMBOURG), at(ESCH));
        assert!(close(d, 16_880.0, 100.0), "{d}");
        let d = distance_m(ll(0.0, 0.0), ll(0.0, -180.0));
        assert!(close(d, PI * EARTH_MEAN_RADIUS_M, 1.0), "{d}");
        assert_eq!(distance_m(at(PARIS), at(PARIS)), 0.0);
        let pole = distance_m(ll(90.0, 0.0), ll(90.0, 120.0));
        assert!(pole < 1e-6, "{pole}");
        // Across the antimeridian the short way is taken.
        let d = distance_m(ll(0.0, 179.5), ll(0.0, -179.5));
        assert!(close(d, 111_195.0, 10.0), "{d}");
    }

    #[test]
    fn bearings_and_compass() {
        let b = initial_bearing(at(PARIS), at(LONDON)).expect("distinct");
        assert!(close(b, 330.0, 0.5), "{b}");
        assert_eq!(Compass::from_bearing(b), Some(Compass::NW));
        let b = initial_bearing(at(NEW_YORK), at(LONDON)).expect("distinct");
        assert!(close(b, 51.2, 0.5), "{b}");
        let b = initial_bearing(ll(0.0, 0.0), ll(1.0, 0.0)).expect("distinct");
        assert!(close(b, 0.0, 1e-9));
        let b = initial_bearing(ll(0.0, 0.0), ll(0.0, 1.0)).expect("distinct");
        assert!(close(b, 90.0, 1e-9));
        let b = initial_bearing(ll(0.0, 0.0), ll(0.0, -1.0)).expect("distinct");
        assert!(close(b, 270.0, 1e-9));
        let b = initial_bearing(ll(0.0, 179.5), ll(0.0, -179.5)).expect("distinct");
        assert!(close(b, 90.0, 1e-9));
        let b = initial_bearing(ll(90.0, 0.0), ll(10.0, 0.0)).expect("distinct");
        assert!(close(b, 180.0, 1e-9));
        assert_eq!(initial_bearing(at(PARIS), at(PARIS)), None);
        let table = [
            (0.0, Compass::N),
            (22.49, Compass::N),
            (22.5, Compass::NE),
            (90.0, Compass::E),
            (135.0, Compass::SE),
            (180.0, Compass::S),
            (220.3, Compass::SW),
            (270.0, Compass::W),
            (315.0, Compass::NW),
            (337.49, Compass::NW),
            (337.5, Compass::N),
            (359.99, Compass::N),
            (-45.0, Compass::NW),
            (720.0, Compass::N),
            (-360.0, Compass::N),
        ];
        for (deg, want) in table {
            assert_eq!(Compass::from_bearing(deg), Some(want), "{deg}");
        }
        assert_eq!(Compass::from_bearing(f64::NAN), None);
        assert_eq!(Compass::from_bearing(f64::INFINITY), None);
        assert_eq!(Compass::SW.to_string(), "SW");
    }

    #[test]
    fn distances_for_people() {
        let table = [
            (0.0, "0 m"),
            (0.4, "0 m"),
            (42.4, "42 m"),
            (99.4, "99 m"),
            (99.6, "100 m"),
            (354.0, "350 m"),
            (355.0, "360 m"),
            (994.9, "990 m"),
            (995.0, "1.0 km"),
            (996.0, "1.0 km"),
            (1_549.0, "1.5 km"),
            (9_949.0, "9.9 km"),
            (9_950.0, "10 km"),
            (15_200.0, "15 km"),
            (5_570_229.0, "5570 km"),
        ];
        for (m, want) in table {
            assert_eq!(format_distance(m), want, "{m}");
        }
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            assert_eq!(format_distance(bad), "unknown");
        }
        assert_eq!(describe_offset(at(LUXEMBOURG), at(ESCH)), "17 km SW");
        assert_eq!(describe_offset(at(ESCH), at(ESCH)), "here");
    }

    #[test]
    fn coordinates_are_formatted() {
        let lux = at(LUXEMBOURG);
        assert_eq!(format_decimal(lux), "49.61160 N  6.13190 E");
        assert_eq!(format_dms(lux), "49d36'41.8\" N  6d07'54.8\" E");
        assert_eq!(format_decimal(at(SYDNEY)), "33.86880 S  151.20930 E");
        assert_eq!(format_dms(at(SYDNEY)), "33d52'07.7\" S  151d12'33.5\" E");
        assert_eq!(format_decimal(at(NEW_YORK)), "40.71280 N  74.00600 W");
        // Rounding to zero never shows S or W; -180 is W.
        assert_eq!(
            format_decimal(ll(-0.000_001, -0.000_001)),
            "0.00000 N  0.00000 E"
        );
        assert_eq!(
            format_dms(ll(-0.000_001, 0.0)),
            "0d00'00.0\" N  0d00'00.0\" E"
        );
        assert_eq!(format_decimal(ll(-90.0, -180.0)), "90.00000 S  180.00000 W");
        // Seconds carry into minutes and degrees.
        assert_eq!(
            format_dms(ll(10.999_999, 0.0)),
            "11d00'00.0\" N  0d00'00.0\" E"
        );
    }

    #[test]
    fn coordinates_are_parsed() {
        let table: &[(&str, f64, f64)] = &[
            ("49.6116 6.1319", 49.6116, 6.1319),
            ("49.6116, 6.1319", 49.6116, 6.1319),
            ("  49.6116,6.1319  ", 49.6116, 6.1319),
            ("49.6116;6.1319", 49.6116, 6.1319),
            ("49.6116 / 6.1319", 49.6116, 6.1319),
            ("-33.8688, 151.2093", -33.8688, 151.2093),
            ("+33.8688 -151.2093", 33.8688, -151.2093),
            ("\u{2212}33.5 \u{2212}70", -33.5, -70.0),
            ("49.6116 N 6.1319 E", 49.6116, 6.1319),
            ("49.6116N 6.1319E", 49.6116, 6.1319),
            ("49.6116 n, 6.1319 w", 49.6116, -6.1319),
            ("N 49.6116 E 6.1319", 49.6116, 6.1319),
            ("S33.8688, E151.2093", -33.8688, 151.2093),
            ("6.1319 E 49.6116 N", 49.6116, 6.1319),
            ("W 74 N 40", 40.0, -74.0),
            ("49 36 41.8 N 6 7 54.8 E", 49.611_611_1, 6.131_888_9),
            (
                "49\u{b0}36'41.8\"N 6\u{b0}7'54.8\"E",
                49.611_611_1,
                6.131_888_9,
            ),
            (
                "49\u{b0} 36' 41.8\" N, 6\u{b0} 7' 54.8\" E",
                49.611_611_1,
                6.131_888_9,
            ),
            ("49d36'41.8\" N  6d07'54.8\" E", 49.611_611_1, 6.131_888_9),
            (
                "49\u{ba}36\u{2032}41.8\u{2033} N 6\u{ba}7\u{2019}54.8\u{201d} E",
                49.611_611_1,
                6.131_888_9,
            ),
            ("49 36 41.8 6 7 54.8", 49.611_611_1, 6.131_888_9),
            ("-33 52 7.7, 151 12 33.5", -33.868_805_6, 151.209_305_6),
            ("49\u{b0}36' 6\u{b0}7'", 49.6, 6.116_666_7),
            ("49 36 6 7", 49.6, 6.116_666_7),
            ("49\u{b0} 36.7' N 6\u{b0} 7.9' E", 49.611_666_7, 6.131_666_7),
            ("49 36 41'' N 6 7 54'' E", 49.611_388_9, 6.131_666_7),
            ("49,6116 6,1319", 49.6116, 6.1319),
            ("49,6116; 6,1319", 49.6116, 6.1319),
            ("48,2", 48.0, 2.0),
            ("90 180", 90.0, -180.0),
            ("-90, -180", -90.0, -180.0),
            ("0 0", 0.0, 0.0),
            (".5 5.", 0.5, 5.0),
            ("0049.5 006", 49.5, 6.0),
        ];
        for &(input, lat, lon) in table {
            let p = parse_coordinates(input).unwrap_or_else(|e| panic!("{input:?}: {e:?}"));
            assert!(close(p.lat(), lat, 1e-6), "{input:?} -> {p:?}");
            assert!(close(p.lon(), lon, 1e-6), "{input:?} -> {p:?}");
        }
    }

    #[test]
    fn bad_coordinates_are_refused() {
        use CoordError::*;
        let long = "1".repeat(MAX_COORD_INPUT + 1);
        let garbage = "\u{1f600}".repeat(100_000);
        let table: &[(&str, CoordError)] = &[
            ("", Empty),
            ("   ", Empty),
            (long.as_str(), TooLong),
            (garbage.as_str(), TooLong),
            ("49.6 6.1 x", UnexpectedCharacter('x')),
            ("49.6 6.1 \u{1b}[2J", UnexpectedCharacter('\u{1b}')),
            ("1e5 3", Letters),
            ("49.6.1 6", BadNumber),
            ("- 6", BadNumber),
            ("49.6 -", BadNumber),
            ("12345678901234567 6", BadNumber),
            (".", BadNumber),
            ("49.6", Layout),
            ("49.6 6.1 3.2", Layout),
            ("1,2,3", Layout),
            ("49.6,", Layout),
            (", 6.1", Layout),
            ("49.6 N", Letters),
            ("49.6 N 6.1 E 3 W", Letters),
            ("49.6 N 6.1 S", Letters),
            ("6.1 E 7.2 W", Letters),
            ("N 49.6 6.1 E", Layout),
            ("49.6 N E 6.1", Layout),
            ("N 49 36, 3 E 6", Layout),
            ("-49.6 N 6.1 E", SignAndLetter),
            ("49.6 N +6.1 E", SignAndLetter),
            ("49 36 41 12 N 6 E", BadDms),
            ("49' 6 E 3 N", BadDms),
            ("49\u{b0}36\"  6\u{b0}7'", BadDms),
            ("49.5 30 N 6 E", BadDms),
            ("49 30.5 10 N 6 E", BadDms),
            ("49 -30 N 6 E", BadDms),
            ("\u{b0}49 6", BadDms),
            ("49\u{b0}\u{b0} 6", BadDms),
            ("49 60 N 6 E", MinutesTooLarge),
            ("49 59 60 N 6 E", SecondsTooLarge),
            ("90.0001 0", LatitudeOutOfRange),
            ("91 N 0 E", LatitudeOutOfRange),
            ("90 0 1 N 0 E", LatitudeOutOfRange),
            ("0 180.0001", LongitudeOutOfRange),
            ("0 S 181 W", LongitudeOutOfRange),
        ];
        for (input, want) in table {
            assert_eq!(parse_coordinates(input), Err(*want), "{input:?}");
            assert!(!want.to_string().is_empty());
        }
        assert_eq!(
            CoordError::UnexpectedCharacter('\u{1b}').to_string(),
            "there is a character that cannot be used in coordinates"
        );
    }

    #[test]
    fn formatted_coordinates_parse_back() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..500 {
            let lat = (next() % 1_800_001) as f64 / 10_000.0 - 90.0;
            let lon = (next() % 3_600_000) as f64 / 10_000.0 - 180.0;
            let p = ll(lat, lon);
            let dec = parse_coordinates(&format_decimal(p)).expect("decimal");
            assert!(
                close(dec.lat(), lat, 1e-5) && close(dec.lon(), lon, 1e-5),
                "{p:?}"
            );
            let dms = parse_coordinates(&format_dms(p)).expect("dms");
            // A tenth of a second is 2.8e-5 degrees.
            assert!(close(dms.lat(), lat, 2e-5), "{p:?} {dms:?}");
            // Longitude may come back as -180 for 180 after rounding.
            let dlon = (dms.lon() - lon).abs();
            assert!(dlon < 2e-5 || (360.0 - dlon) < 2e-5, "{p:?} {dms:?}");
        }
    }

    #[test]
    fn random_coordinate_input_never_panics() {
        let alphabet: Vec<char> =
            "0123456789.,;/ +-NSEWnsewdD'\"\u{b0}\u{2032}\u{2033}xé\u{2212}\t"
                .chars()
                .collect();
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..5_000 {
            let len = (next() % 40) as usize;
            let s: String = (0..len)
                .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                .collect();
            if let Ok(p) = parse_coordinates(&s) {
                assert!(p.lat().abs() <= 90.0 && p.lon().abs() <= 180.0, "{s:?}");
            }
        }
    }

    #[test]
    fn scale_bar_lengths() {
        let expected = [
            (5.0, 20, "100 m"),
            (10.0, 20, "200 m"),
            (20.0, 10, "200 m"),
            (50.0, 20, "1 km"),
            (100.0, 20, "2 km"),
            (200.0, 10, "2 km"),
            (500.0, 20, "10 km"),
            (1_000.0, 20, "20 km"),
            (2_000.0, 10, "20 km"),
            (5_000.0, 20, "100 km"),
            (10_000.0, 20, "200 km"),
            (20_000.0, 10, "200 km"),
            (50_000.0, 20, "1000 km"),
            (100_000.0, 20, "2000 km"),
            (200_000.0, 10, "2000 km"),
        ];
        for (scale, cells, label) in expected {
            let bar = scale_bar(scale, 8, 20).expect("fits");
            assert_eq!((bar.cells, bar.label.as_str()), (cells, label), "{scale}");
            assert_eq!(bar.metres as f64, f64::from(bar.cells) * scale);
        }
        // Every scale of the view's range gets a bar within 8..=20 cells.
        let mut s = MIN_SCALE;
        while s <= MAX_SCALE {
            let bar = scale_bar(s, 8, 20).expect("fits");
            assert!((8..=20).contains(&bar.cells), "{s}");
            s *= 1.07;
        }
        let odd = scale_bar(37.0, 8, 20).expect("fits");
        assert_eq!(
            (odd.cells, odd.metres, odd.label.as_str()),
            (14, 500, "500 m")
        );
        assert_eq!(scale_bar(f64::NAN, 8, 20), None);
        assert_eq!(scale_bar(0.0, 8, 20), None);
        assert_eq!(scale_bar(-5.0, 8, 20), None);
        assert_eq!(scale_bar(5.0, 0, 20), None);
        assert_eq!(scale_bar(5.0, 21, 20), None);
        assert_eq!(scale_bar(5.0, 11, 11), None);
        assert_eq!(scale_bar(1e12, 8, 20), None);
    }
}
