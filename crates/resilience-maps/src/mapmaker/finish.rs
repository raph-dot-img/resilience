//! `resilience-mapmaker finish`: tippecanoe's MBTiles, the side file of
//! rows and the coverage `.poly` in, one `resilience-1` pack out.
//!
//! Determinism (ADR 0016 §2, "the finish step fixes the rest"): the pack
//! is a new database with a fixed page size and application id; tiles are
//! copied in (zoom, column, row) order; rows are sorted by every column
//! before they get their ids; metadata is written in key order and holds
//! no time, path or host; the FTS index is rebuilt and optimised, the
//! R*Tree filled in id order; then `VACUUM`. No hash map or thread
//! decides any order.
//!
//! Memory is bounded: rows and street segments are staged in temporary
//! SQLite tables on disk (`temp_store=FILE`, a 64 MiB cache) and grouped
//! and sorted by SQLite, so the only data held in memory is the list of
//! settlements used for `near` (about 60 bytes each plus the name) and
//! one tile at a time.
//!
//! Every tile is decompressed and decoded with the viewer's own bounded
//! decoder (`inflate`, `mvt`): a tile the viewer would refuse fails the
//! build, naming the tile. At the end the pack is opened with the
//! hardened reader ([`crate::pack::Pack`]) and must offer search, nearby
//! and coverage and pass `quick_check`.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use resilience_core::maps::geo::distance_m;
use resilience_core::maps::mvt;
use resilience_core::maps::style::{Layer, PeakClass};
use resilience_core::packs::valid_data_date;
use rusqlite::{Connection, OpenFlags, params};

use super::classify::read_line_bounded;
use super::fold::search_fold;
use super::poly::{self, PolyRing};
use super::rows::{Kind, Row};
use super::tags::PLACES;
use crate::inflate;
use crate::metadata;
use crate::pack::Pack;
use crate::profile::{self, Micro, MicroBox};

/// Page size of every pack.
pub const PAGE_SIZE: u32 = 4096;
/// MBTiles 1.3: `application_id` = "MPBX".
pub const APPLICATION_ID: i32 = 0x4d50_4258;
/// `near` is the nearest settlement within this distance (ADR §4).
pub const NEAR_M: f64 = 10_000.0;
/// Zoom of the `center` metadata.
pub const CENTER_ZOOM: u8 = 10;
/// Longest line of the side file.
const MAX_ROW_LINE: usize = 64 * 1024;
/// Grid cell of the settlement index, in microdegrees (0.1°, 11 km of
/// latitude: more than [`NEAR_M`]).
const CELL: i32 = 100_000;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error(format!("SQLite: {e}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(format!("I/O: {e}"))
    }
}

fn fail<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

/// What the pack says about itself (metadata), from the region's Nix
/// description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Describe {
    pub name: String,
    pub description: Option<String>,
    /// `YYYY-MM-DD`, from the PBF header.
    pub data_date: String,
    /// The input file and its SHA-256.
    pub source: String,
    pub coverage: String,
    pub license: String,
    pub attribution: String,
    pub builder: String,
}

pub struct Inputs<'a> {
    pub tiles: &'a Path,
    pub rows: &'a Path,
    pub poly: &'a Path,
}

/// The largest tiles seen, for the build log.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TileStats {
    /// (zoom, count, compressed bytes) for each zoom present.
    pub by_zoom: Vec<(u8, u64, u64)>,
    /// (bytes, z, x, y with y in TMS).
    pub largest_compressed: (usize, u8, u32, u32),
    pub largest_decompressed: (usize, u8, u32, u32),
    pub most_features: (usize, u8, u32, u32),
    pub most_vertices: (usize, u8, u32, u32),
    /// Most features in one layer of one tile, and the layer.
    pub most_layer_features: (usize, String),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    pub tiles: TileStats,
    pub rows_by_kind: [u64; 4],
    pub rtree: u64,
    pub segments: u64,
    pub coverage_rings: usize,
    pub coverage_points: usize,
    /// (object, bytes) from SQLite's `dbstat`, largest first.
    pub space: Vec<(String, u64)>,
    pub file_size: u64,
}

impl Stats {
    pub fn report(&self) -> String {
        let t = &self.tiles;
        let mut s = String::new();
        for (z, n, b) in &t.by_zoom {
            s.push_str(&format!("finish: z{z}: {n} tiles, {b} bytes\n"));
        }
        let at = |v: &(usize, u8, u32, u32)| format!("{} at {}/{}/{} (TMS)", v.0, v.1, v.2, v.3);
        s.push_str(&format!(
            "finish: largest tile in bytes, compressed: {} (limit {}); decompressed: {} (limit {})\n",
            at(&t.largest_compressed),
            inflate::MAX_COMPRESSED,
            at(&t.largest_decompressed),
            inflate::MAX_DECOMPRESSED
        ));
        s.push_str(&format!(
            "finish: most features in a tile: {} (limit {}); most vertices: {} (limit {} geometry integers, 2 or more per vertex); most features in one layer: {} ({})\n",
            at(&t.most_features),
            mvt::MAX_FEATURES,
            at(&t.most_vertices),
            mvt::MAX_GEOMETRY_INTEGERS,
            t.most_layer_features.0,
            t.most_layer_features.1
        ));
        s.push_str(&format!(
            "finish: rows settlement={} street={} poi={} natural={} (from {} segments), rtree={}\n",
            self.rows_by_kind[0],
            self.rows_by_kind[1],
            self.rows_by_kind[2],
            self.rows_by_kind[3],
            self.segments,
            self.rtree
        ));
        s.push_str(&format!(
            "finish: coverage {} rings, {} points\n",
            self.coverage_rings, self.coverage_points
        ));
        for (name, bytes) in &self.space {
            s.push_str(&format!("finish: space {name}: {bytes} bytes\n"));
        }
        s.push_str(&format!("finish: file size {} bytes\n", self.file_size));
        s
    }
}

// ---------------------------------------------------------------------------
// Settlements for `near`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Settlement {
    at_lon: i32,
    at_lat: i32,
    rank: i64,
    name: String,
}

/// The settlements a row can be "near": the four drawn classes.
pub struct NearIndex {
    list: Vec<Settlement>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

fn cell_of(m: Micro) -> (i32, i32) {
    (m.lon.div_euclid(CELL), m.lat.div_euclid(CELL))
}

impl NearIndex {
    /// From (position, rank, name); the order given does not matter.
    pub fn new(mut items: Vec<(Micro, i64, String)>) -> NearIndex {
        items.sort_by(|a, b| (a.0.lon, a.0.lat, a.1, &a.2).cmp(&(b.0.lon, b.0.lat, b.1, &b.2)));
        items.dedup();
        let list: Vec<Settlement> = items
            .into_iter()
            .map(|(m, rank, name)| Settlement {
                at_lon: m.lon,
                at_lat: m.lat,
                rank,
                name,
            })
            .collect();
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, s) in list.iter().enumerate() {
            let c = cell_of(Micro {
                lon: s.at_lon,
                lat: s.at_lat,
            });
            grid.entry(c)
                .or_default()
                .push(u32::try_from(i).unwrap_or(u32::MAX));
        }
        NearIndex { list, grid }
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The index and name of the nearest settlement within [`NEAR_M`] of
    /// `at`, other than those named `not_named`. Ties: lower rank, then
    /// name, then position (the list order). Grid cells are visited in a
    /// fixed order and the minimum is taken by a total order, so the
    /// result never depends on the hash map.
    pub fn nearest(&self, at: Micro, not_named: Option<&str>) -> Option<(usize, &str)> {
        let (cx, cy) = cell_of(at);
        let cos = at.lat_degrees().to_radians().cos().max(0.01);
        let lon_deg = NEAR_M / (111_320.0 * cos);
        let reach = ((lon_deg * profile::MICRO / f64::from(CELL)).ceil() as i32).clamp(1, 1800);
        let here = at.lat_lon();
        let mut best: Option<(f64, usize)> = None;
        for dy in -1..=1 {
            for dx in -reach..=reach {
                // Longitude wraps around the antimeridian.
                let x = (cx + dx + 1800).rem_euclid(3600) - 1800;
                let Some(cell) = self.grid.get(&(x, cy + dy)) else {
                    continue;
                };
                for &i in cell {
                    let i = i as usize;
                    let s = &self.list[i];
                    if not_named.is_some_and(|n| n == s.name) {
                        continue;
                    }
                    let d = distance_m(
                        here,
                        Micro {
                            lon: s.at_lon,
                            lat: s.at_lat,
                        }
                        .lat_lon(),
                    );
                    if d > NEAR_M {
                        continue;
                    }
                    let better = match best {
                        None => true,
                        Some((bd, bi)) => {
                            let b = &self.list[bi];
                            (d, s.rank, &s.name, i)
                                .partial_cmp(&(bd, b.rank, &b.name, bi))
                                .is_some_and(|o| o.is_lt())
                        }
                    };
                    if better {
                        best = Some((d, i));
                    }
                }
            }
        }
        best.map(|(_, i)| (i, self.list[i].name.as_str()))
    }
}

// ---------------------------------------------------------------------------
// The side file
// ---------------------------------------------------------------------------

fn for_each_row(path: &Path, mut f: impl FnMut(Row) -> Result<(), Error>) -> Result<(), Error> {
    let mut r = BufReader::new(File::open(path)?);
    let mut buf = Vec::new();
    let mut line = 0u64;
    while let Some(fits) = read_line_bounded(&mut r, &mut buf, MAX_ROW_LINE)? {
        line += 1;
        if !fits {
            return fail(format!("rows line {line}: too long"));
        }
        if buf.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let row: Row =
            serde_json::from_slice(&buf).map_err(|e| Error(format!("rows line {line}: {e}")))?;
        row.check()
            .map_err(|e| Error(format!("rows line {line}: {e}")))?;
        f(row)?;
    }
    Ok(())
}

fn is_near_class(class: &str) -> bool {
    PLACES.iter().any(|(v, _)| *v == class)
}

/// A group key for a segment without a settlement nearby: its 0.1° cell,
/// as a negative number (settlement indexes are not negative).
fn lonely_group(at: Micro) -> i64 {
    let (x, y) = cell_of(at);
    -1 - ((i64::from(x) + 1800) * 2000 + (i64::from(y) + 900))
}

// ---------------------------------------------------------------------------
// The pack
// ---------------------------------------------------------------------------

fn check_metadata_value(key: &str, value: &str) -> Result<(), Error> {
    if let Some((_, max)) = metadata::KEYS.iter().find(|(k, _)| *k == key)
        && value.len() > *max
    {
        return fail(format!("metadata {key} is longer than {max} bytes"));
    }
    if value.trim().is_empty() || value.chars().any(crate::text::is_unsafe) {
        return fail(format!("metadata {key} is empty or not printable"));
    }
    Ok(())
}

fn create(out: &Path) -> Result<Connection, Error> {
    if std::fs::symlink_metadata(out).is_ok() {
        return fail(format!("{} exists already", out.display()));
    }
    let conn = Connection::open_with_flags(
        out,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    conn.execute_batch(&format!(
        "PRAGMA page_size = {PAGE_SIZE};
         PRAGMA auto_vacuum = NONE;
         PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         PRAGMA temp_store = FILE;
         PRAGMA cache_size = -65536;
         PRAGMA application_id = {APPLICATION_ID};"
    ))?;
    conn.execute_batch(
        "CREATE TABLE metadata (name TEXT, value TEXT);
         CREATE UNIQUE INDEX name ON metadata (name);
         CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data BLOB);
         CREATE UNIQUE INDEX tile_index ON tiles (zoom_level, tile_column, tile_row);",
    )?;
    // The profile's tables, verbatim (the reader compares them byte for
    // byte with what this same SQLite makes of these statements).
    for sql in profile::TABLES {
        conn.execute_batch(sql)?;
    }
    Ok(conn)
}

fn write_coverage(conn: &Connection, rings: &[PolyRing]) -> Result<(), Error> {
    let mut st =
        conn.prepare("INSERT INTO rs_coverage (ring, hole, points) VALUES (?1, ?2, ?3)")?;
    for (i, r) in rings.iter().enumerate() {
        st.execute(params![
            i64::try_from(i + 1).unwrap_or(i64::MAX),
            i64::from(r.hole),
            profile::encode_ring(&r.points)
        ])?;
    }
    Ok(())
}

/// Stage every row, merge segments, write `rs_place`, its FTS index and
/// the R*Tree.
fn write_places(conn: &Connection, rows: &Path, stats: &mut Stats) -> Result<(), Error> {
    // Pass 1: the settlements.
    let mut items = Vec::new();
    for_each_row(rows, |r| {
        if r.kind == Kind::Settlement && is_near_class(&r.class) && !r.name.is_empty() {
            items.push((r.at(), r.rank, r.name));
        }
        Ok(())
    })?;
    let near = NearIndex::new(items);

    // Pass 2: stage.
    conn.execute_batch(
        "CREATE TEMP TABLE seg (kind INTEGER, name TEXT, grp INTEGER, near TEXT, cls TEXT,
                                rank INTEGER, lon INTEGER, lat INTEGER,
                                w INTEGER, s INTEGER, e INTEGER, n INTEGER);
         CREATE TEMP TABLE place (kind INTEGER, class TEXT, name TEXT, near TEXT,
                                  lon INTEGER, lat INTEGER, rank INTEGER,
                                  w INTEGER, s INTEGER, e INTEGER, n INTEGER);
         BEGIN;",
    )?;
    {
        let mut seg = conn.prepare(
            "INSERT INTO temp.seg VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        let mut place = conn.prepare(
            "INSERT INTO temp.place VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for_each_row(rows, |r| {
            let not_named = (r.kind == Kind::Settlement).then_some(r.name.as_str());
            let found = near.nearest(r.at(), not_named);
            let near_name = found.map(|(_, n)| n);
            let [w, s, e, n] = match r.bbox {
                Some(b) => b.map(Some),
                None => [None; 4],
            };
            if r.merge {
                let grp = match found {
                    Some((i, _)) => i64::try_from(i).unwrap_or(i64::MAX),
                    None => lonely_group(r.at()),
                };
                seg.execute(params![
                    r.kind.order(),
                    r.name,
                    grp,
                    near_name,
                    r.class,
                    r.rank,
                    r.lon,
                    r.lat,
                    w,
                    s,
                    e,
                    n
                ])?;
                stats.segments += 1;
            } else {
                place.execute(params![
                    r.kind.order(),
                    r.class,
                    r.name,
                    near_name,
                    r.lon,
                    r.lat,
                    r.rank,
                    w,
                    s,
                    e,
                    n
                ])?;
            }
            Ok(())
        })?;
    }
    // One row per kind, name and nearest settlement (or 0.1° cell when
    // none is within 10 km): the union of the boxes, the best (lowest)
    // rank and its class, and as position the segment point nearest the
    // box's centre (ties: west, then south).
    conn.execute_batch(
        "CREATE INDEX temp.seg_group ON seg (kind, name, grp, rank, cls);
         INSERT INTO temp.place (kind, class, name, near, lon, lat, rank, w, s, e, n)
         SELECT kind, cls, name, near, lon, lat, grank, gw, gs, ge, gn FROM (
           SELECT x.kind, x.name, x.near, x.lon, x.lat, g.grank, g.gw, g.gs, g.ge, g.gn,
             (SELECT y.cls FROM temp.seg y
               WHERE y.kind = x.kind AND y.name = x.name AND y.grp = x.grp
               ORDER BY y.rank, y.cls LIMIT 1) AS cls,
             row_number() OVER (
               PARTITION BY x.kind, x.name, x.grp
               ORDER BY (x.lon - (g.gw + g.ge) / 2) * (x.lon - (g.gw + g.ge) / 2)
                      + (x.lat - (g.gs + g.gn) / 2) * (x.lat - (g.gs + g.gn) / 2),
                        x.lon, x.lat) AS rn
           FROM temp.seg x JOIN (
             SELECT kind, name, grp, min(rank) AS grank,
                    min(w) AS gw, min(s) AS gs, max(e) AS ge, max(n) AS gn
             FROM temp.seg GROUP BY kind, name, grp) g
           ON g.kind = x.kind AND g.name = x.name AND g.grp = x.grp)
         WHERE rn = 1;
         COMMIT;
         BEGIN;",
    )?;

    // Final rows, in an order fixed by every column.
    let kinds = [Kind::Settlement, Kind::Street, Kind::Poi, Kind::Natural];
    let peaks: Vec<&str> = PeakClass::ALL.iter().map(|p| p.as_str()).collect();
    {
        let mut read = conn.prepare(
            "SELECT DISTINCT kind, class, name, near, lon, lat, rank, w, s, e, n FROM temp.place
             ORDER BY kind, rank, name, near, lon, lat, class, w, s, e, n",
        )?;
        let mut insert = conn.prepare(
            "INSERT INTO rs_place (id, kind, class, name, fold, near, lon, lat, rank, bbox)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        let mut rtree = conn.prepare(
            "INSERT INTO rs_poi_rtree (id, min_lon, max_lon, min_lat, max_lat)
             VALUES (?1, ?2, ?2, ?3, ?3)",
        )?;
        let mut rows = read.query([])?;
        let mut id: i64 = 0;
        while let Some(r) = rows.next()? {
            id += 1;
            let k: i64 = r.get(0)?;
            let kind = usize::try_from(k)
                .ok()
                .and_then(|k| kinds.get(k))
                .copied()
                .ok_or(Error("bad staged kind".into()))?;
            let class: String = r.get(1)?;
            let name: String = r.get(2)?;
            let near_name: Option<String> = r.get(3)?;
            let (lon, lat): (i32, i32) = (r.get(4)?, r.get(5)?);
            let rank: i64 = r.get(6)?;
            let b: [Option<i32>; 4] = [r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?];
            let bbox = match b {
                [Some(w), Some(s), Some(e), Some(n)] => Some(profile::encode_bbox(MicroBox {
                    south_west: Micro::new(w, s).ok_or(Error("bad box".into()))?,
                    north_east: Micro::new(e, n).ok_or(Error("bad box".into()))?,
                })),
                _ => None,
            };
            insert.execute(params![
                id,
                kind.as_str(),
                class,
                name,
                search_fold(&name),
                near_name,
                lon,
                lat,
                rank,
                bbox
            ])?;
            stats.rows_by_kind[usize::try_from(k).unwrap_or(0)] += 1;
            if kind == Kind::Poi || (kind == Kind::Natural && peaks.contains(&class.as_str())) {
                rtree.execute(params![id, lon, lat])?;
                stats.rtree += 1;
            }
        }
    }
    conn.execute_batch(
        "INSERT INTO rs_place_fts (rs_place_fts) VALUES ('rebuild');
         INSERT INTO rs_place_fts (rs_place_fts) VALUES ('optimize');
         COMMIT;
         DROP TABLE temp.seg;
         DROP TABLE temp.place;",
    )?;
    Ok(())
}

/// What finish takes from tippecanoe's metadata.
#[derive(Debug, Default)]
struct TileMeta {
    bounds: Option<[f64; 4]>,
    vector_layers: Option<String>,
}

fn read_tile_meta(src: &Connection) -> Result<TileMeta, Error> {
    let mut m = TileMeta::default();
    let mut st = src.prepare("SELECT name, value FROM metadata ORDER BY name")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let (Ok(name), Ok(value)) = (r.get::<_, String>(0), r.get::<_, String>(1)) else {
            continue;
        };
        match name.as_str() {
            "bounds" => {
                let v: Vec<f64> = value
                    .split(',')
                    .filter_map(|x| x.trim().parse().ok())
                    .collect();
                if let [w, s, e, n] = v[..]
                    && [w, s, e, n].iter().all(|x| x.is_finite())
                    && w.abs() <= 180.0
                    && e.abs() <= 180.0
                    && s.abs() <= 90.0
                    && n.abs() <= 90.0
                    && w < e
                    && s < n
                {
                    m.bounds = Some([w, s, e, n]);
                }
            }
            "json" => {
                let j: serde_json::Value = serde_json::from_str(&value)
                    .map_err(|e| Error(format!("tippecanoe json metadata: {e}")))?;
                if let Some(v) = j.get("vector_layers") {
                    let mut out = serde_json::Map::new();
                    out.insert("vector_layers".into(), v.clone());
                    m.vector_layers =
                        Some(serde_json::to_string(&out).map_err(|e| Error(format!("json: {e}")))?);
                }
            }
            _ => {}
        }
    }
    Ok(m)
}

fn worse(cur: &mut (usize, u8, u32, u32), v: usize, z: u8, x: u32, y: u32) {
    if v > cur.0 {
        *cur = (v, z, x, y);
    }
}

/// Copy, check and count the tiles. Returns the zoom range.
fn copy_tiles(
    src: &Connection,
    dst: &Connection,
    stats: &mut TileStats,
) -> Result<(u8, u8), Error> {
    let mut read = src.prepare(
        "SELECT zoom_level, tile_column, tile_row, tile_data FROM tiles
         ORDER BY zoom_level, tile_column, tile_row",
    )?;
    let mut insert = dst.prepare(
        "INSERT INTO tiles (zoom_level, tile_column, tile_row, tile_data) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let names: Vec<&str> = Layer::ALL.iter().map(|l| l.as_str()).collect();
    let mut rows = read.query([])?;
    let mut by_zoom: BTreeMap<u8, (u64, u64)> = BTreeMap::new();
    dst.execute_batch("BEGIN")?;
    while let Some(r) = rows.next()? {
        let (z, x, y): (i64, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
        let (Ok(z), Ok(x), Ok(y)) = (u8::try_from(z), u32::try_from(x), u32::try_from(y)) else {
            return fail(format!("tile {z}/{x}/{y}: numbers out of range"));
        };
        if z > resilience_core::maps::style::MAX_TILE_ZOOM
            || u64::from(x) >= 1u64 << z
            || u64::from(y) >= 1u64 << z
        {
            return fail(format!("tile {z}/{x}/{y}: not a tile of zoom {z}"));
        }
        let blob: Vec<u8> = r.get(3)?;
        let raw = inflate::decompress_tile(&blob)
            .map_err(|e| Error(format!("tile {z}/{x}/{y} (TMS): {e}")))?;
        let tile = mvt::decode(&raw).map_err(|e| Error(format!("tile {z}/{x}/{y} (TMS): {e}")))?;
        if tile.skipped_layers() > 0 {
            return fail(format!("tile {z}/{x}/{y}: a layer is not version 2"));
        }
        let (mut features, mut vertices) = (0usize, 0usize);
        for l in tile.layers() {
            if !names.contains(&l.name()) {
                return fail(format!("tile {z}/{x}/{y}: unknown layer {:?}", l.name()));
            }
            if l.skipped_features() > 0 {
                return fail(format!("tile {z}/{x}/{y}: features of unknown geometry"));
            }
            features += l.len();
            vertices += l.vertex_count();
            if l.len() > stats.most_layer_features.0 {
                stats.most_layer_features = (l.len(), format!("{} in {z}/{x}/{y}", l.name()));
            }
        }
        worse(&mut stats.largest_compressed, blob.len(), z, x, y);
        worse(&mut stats.largest_decompressed, raw.len(), z, x, y);
        worse(&mut stats.most_features, features, z, x, y);
        worse(&mut stats.most_vertices, vertices, z, x, y);
        let e = by_zoom.entry(z).or_default();
        e.0 += 1;
        e.1 += blob.len() as u64;
        insert.execute(params![z, x, y, blob])?;
    }
    dst.execute_batch("COMMIT")?;
    stats.by_zoom = by_zoom.iter().map(|(z, (n, b))| (*z, *n, *b)).collect();
    match (by_zoom.keys().next(), by_zoom.keys().next_back()) {
        (Some(a), Some(b)) => Ok((*a, *b)),
        _ => fail("tippecanoe wrote no tiles"),
    }
}

/// The map's opening position: the most important settlement inside the
/// coverage (lowest rank, then name, then position), else the middle of
/// the bounds.
fn center(conn: &Connection, rings: &[PolyRing], bounds: [f64; 4]) -> Result<(f64, f64), Error> {
    let mut st = conn.prepare(
        "SELECT lon, lat FROM rs_place WHERE kind = 'settlement' ORDER BY rank, name, lon, lat",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        if let Some(m) = Micro::new(r.get(0)?, r.get(1)?)
            && poly::contains(rings, m)
        {
            return Ok((m.lon_degrees(), m.lat_degrees()));
        }
    }
    Ok(((bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0))
}

fn space(conn: &Connection) -> Vec<(String, u64)> {
    let Ok(mut st) = conn
        .prepare("SELECT name, sum(pgsize) AS b FROM dbstat GROUP BY name ORDER BY b DESC, name")
    else {
        return Vec::new();
    };
    st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map(|it| {
            it.filter_map(Result::ok)
                .map(|(n, b)| (n, u64::try_from(b).unwrap_or(0)))
                .collect()
        })
        .unwrap_or_default()
}

/// Run `finish`, writing the pack at `out` (which must not exist).
pub fn run(inputs: &Inputs<'_>, d: &Describe, out: &Path) -> Result<Stats, Error> {
    if !valid_data_date(&d.data_date) {
        return fail(format!("data date {:?} is not YYYY-MM-DD", d.data_date));
    }
    let mut stats = Stats::default();
    let poly_text = std::fs::read_to_string(inputs.poly)?;
    let rings = poly::parse(&poly_text).map_err(|e| Error(format!("coverage .poly: {e}")))?;
    stats.coverage_rings = rings.len();
    stats.coverage_points = rings.iter().map(|r| r.points.len()).sum();

    let src = Connection::open_with_flags(
        inputs.tiles,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let tile_meta = read_tile_meta(&src)?;

    let conn = create(out)?;
    conn.execute_batch("BEGIN")?;
    write_coverage(&conn, &rings)?;
    conn.execute_batch("COMMIT")?;
    write_places(&conn, inputs.rows, &mut stats)?;
    let (minzoom, maxzoom) = copy_tiles(&src, &conn, &mut stats.tiles)?;
    drop(src);

    let bounds = match tile_meta.bounds {
        Some(b) => b,
        None => {
            let pts = rings.iter().flat_map(|r| r.points.iter());
            let (mut w, mut s, mut e, mut n) = (180.0f64, 90.0f64, -180.0f64, -90.0f64);
            for p in pts {
                w = w.min(p.lon_degrees());
                e = e.max(p.lon_degrees());
                s = s.min(p.lat_degrees());
                n = n.max(p.lat_degrees());
            }
            [w, s, e, n]
        }
    };
    let (clon, clat) = center(&conn, &rings, bounds)?;
    let mut meta: BTreeMap<&str, String> = BTreeMap::new();
    meta.insert("name", d.name.clone());
    meta.insert("format", "pbf".into());
    meta.insert("type", "baselayer".into());
    meta.insert("minzoom", minzoom.to_string());
    meta.insert("maxzoom", maxzoom.to_string());
    meta.insert(
        "bounds",
        format!(
            "{:.6},{:.6},{:.6},{:.6}",
            bounds[0], bounds[1], bounds[2], bounds[3]
        ),
    );
    meta.insert("center", format!("{clon:.6},{clat:.6},{CENTER_ZOOM}"));
    meta.insert("attribution", d.attribution.clone());
    if let Some(desc) = &d.description {
        meta.insert("description", desc.clone());
    }
    if let Some(j) = tile_meta.vector_layers {
        meta.insert("json", j);
    }
    meta.insert("resilience_profile", profile::PROFILE_VALUE.into());
    meta.insert("resilience_data_date", d.data_date.clone());
    meta.insert("resilience_source", d.source.clone());
    meta.insert("resilience_coverage", d.coverage.clone());
    meta.insert("resilience_license", d.license.clone());
    meta.insert("resilience_builder", d.builder.clone());
    {
        let mut st = conn.prepare("INSERT INTO metadata (name, value) VALUES (?1, ?2)")?;
        conn.execute_batch("BEGIN")?;
        for (k, v) in &meta {
            if *k != "json" {
                check_metadata_value(k, v)?;
            }
            st.execute(params![k, v])?;
        }
        conn.execute_batch("COMMIT")?;
    }
    conn.execute_batch("VACUUM")?;
    stats.space = space(&conn);
    conn.close().map_err(|(_, e)| Error::from(e))?;
    stats.file_size = std::fs::metadata(out)?.len();

    // The pack must open with the viewer's reader, with every feature.
    let pack = Pack::open(out).map_err(|e| Error(format!("the reader refuses the pack: {e}")))?;
    let caps = pack.capabilities();
    for (what, f) in [
        ("search", &caps.search),
        ("nearby", &caps.nearby),
        ("coverage", &caps.coverage),
    ] {
        if let Err(e) = f {
            return fail(format!("the reader finds no {what} in the pack: {e}"));
        }
    }
    pack.quick_check()
        .map_err(|e| Error(format!("quick_check: {e}")))?;
    if pack.metadata().data_date.as_deref() != Some(d.data_date.as_str()) {
        return fail("the reader reads another data date");
    }
    Ok(stats)
}

#[cfg(test)]
mod tests;
