//! Synthetic map packs for tests, written at test time into temporary
//! directories (no binary files in Git). Shared by the unit tests and by
//! `tests/cli.rs`, which includes this file with `#[path]`; it therefore
//! names the library as `resilience_maps` (see `lib.rs`).

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use resilience_core::maps::geo::{LatLon, TileId, TmsTile, project};
use resilience_core::maps::mvt_encode::{
    LayerBuilder, TestValue, feature, lines, points, polygon, tile, value,
};
use resilience_maps::inflate::gzip;
use resilience_maps::profile::{self, Micro};
use rusqlite::Connection;

/// A directory removed (made writable first) when dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("resilience-maps-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test dir");
        TempDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    pub fn set_read_only(&self, read_only: bool) {
        let mode = if read_only { 0o555 } else { 0o755 };
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(mode))
            .expect("chmod test dir");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Every entry of `dir`: name, size, modification time (ns). Symlinks are
/// not followed.
pub fn listing(dir: &Path) -> Vec<(String, u64, i64, i64)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .expect("read test dir")
        .map(|e| {
            let e = e.expect("dir entry");
            let m = std::fs::symlink_metadata(e.path()).expect("stat");
            (
                e.file_name().to_string_lossy().into_owned(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
            )
        })
        .collect();
    v.sort();
    v
}

/// Run `f` and assert that `dir` holds exactly the same files afterwards.
pub fn unchanged<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    let before = listing(dir);
    let out = f();
    assert_eq!(
        listing(dir),
        before,
        "something was written beside the pack"
    );
    out
}

/// An uncompressed vector tile: a `places` point named `label`, a road.
pub fn sample_tile(label: &str) -> Vec<u8> {
    let mut places = LayerBuilder::new("places");
    places.keys = vec!["name".into(), "class".into()];
    places.values = vec![
        value(&TestValue::Str(label)),
        value(&TestValue::Str("town")),
    ];
    places.features = vec![feature(Some(1), 1, &[0, 0, 1, 1], &points(&[(2048, 2048)]))];
    let mut roads = LayerBuilder::new("roads");
    roads.keys = vec!["class".into()];
    roads.values = vec![value(&TestValue::Str("primary"))];
    roads.features = vec![feature(
        Some(2),
        2,
        &[0, 0],
        &lines(&[&[(0, 0), (4096, 4096)]]),
    )];
    tile(&[places.encode(), roads.encode()])
}

/// The tiles of the valid pack, in TMS numbering, with their labels.
pub fn sample_tiles() -> Vec<(TmsTile, String)> {
    [(0, 0, 0), (1, 0, 0), (1, 1, 1), (2, 2, 2)]
        .into_iter()
        .map(|(z, x, y)| {
            (
                TmsTile::new(z, x, y).expect("valid tile"),
                format!("T{z}-{x}-{y}"),
            )
        })
        .collect()
}

pub const METADATA: [(&str, &str); 14] = [
    ("name", "Testland"),
    ("format", "pbf"),
    ("minzoom", "0"),
    ("maxzoom", "2"),
    ("bounds", "5.70000,49.40000,6.60000,50.20000"),
    ("center", "6.13190,49.61160,2"),
    ("attribution", "(c) OpenStreetMap contributors"),
    ("description", "Synthetic test pack"),
    ("json", "{\"vector_layers\":[]}"),
    ("resilience_profile", "1"),
    ("resilience_data_date", "2026-01-01"),
    (
        "resilience_source",
        "testland-260101.osm.pbf sha256:0123456789abcdef",
    ),
    ("resilience_coverage", "Testland, whole country"),
    ("resilience_license", "ODbL-1.0"),
];

/// The coverage ring of the valid pack.
pub fn sample_ring() -> Vec<Micro> {
    [
        (5_700_000, 49_400_000),
        (6_600_000, 49_400_000),
        (6_600_000, 50_200_000),
        (5_700_000, 50_200_000),
    ]
    .into_iter()
    .map(|(lon, lat)| Micro::new(lon, lat).expect("in the world"))
    .collect()
}

/// The schema tippecanoe writes.
pub const TIPPECANOE_SCHEMA: &str = "
    CREATE TABLE metadata (name text, value text);
    CREATE UNIQUE INDEX name on metadata (name);
    CREATE TABLE tiles (zoom_level integer, tile_column integer, tile_row integer, tile_data blob);
    CREATE UNIQUE INDEX tile_index on tiles (zoom_level, tile_column, tile_row);";

/// Write the `rs_` tables with `statements` (normally [`profile::TABLES`])
/// and a few rows.
pub fn write_rs_tables(c: &Connection, statements: &[&str]) {
    for sql in statements {
        c.execute_batch(sql).expect("create rs_ table");
    }
    c.execute(
        "INSERT INTO rs_coverage (ring, hole, points) VALUES (1, 0, ?1)",
        [profile::encode_ring(&sample_ring())],
    )
    .expect("coverage");
    c.execute_batch(
        "INSERT INTO rs_place VALUES (1, 'settlement', 'town', 'Testville', 'testville', NULL, 6131900, 49611600, 1, NULL);
         INSERT INTO rs_place VALUES (2, 'poi', 'pharmacy', 'Pharmacie du Test', 'pharmacie du test', 'Testville', 6132000, 49611000, 1, NULL);
         INSERT INTO rs_place VALUES (3, 'street', 'residential', 'Rue de l''Essai', 'rue de l''essai', 'Testville', 6131000, 49612000, 1, NULL);
         INSERT INTO rs_place_fts (rowid, name) SELECT id, name FROM rs_place;
         INSERT INTO rs_poi_rtree VALUES (2, 6132000, 6132000, 49611000, 49611000);",
    )
    .expect("places");
}

/// Write the valid pack at `path`, then let `alter` change it.
pub fn write_pack_with(path: &Path, alter: impl FnOnce(&Connection)) {
    let c = Connection::open(path).expect("create pack");
    c.execute_batch(TIPPECANOE_SCHEMA).expect("schema");
    for (k, v) in METADATA {
        c.execute("INSERT INTO metadata (name, value) VALUES (?1, ?2)", [k, v])
            .expect("metadata");
    }
    for (t, label) in sample_tiles() {
        c.execute(
            "INSERT INTO tiles VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![t.zoom(), t.column(), t.row(), gzip(&sample_tile(&label))],
        )
        .expect("tile");
    }
    write_rs_tables(&c, &profile::TABLES);
    alter(&c);
    c.close().expect("close pack");
}

/// Write the valid pack at `path`.
pub fn write_pack(path: &Path) {
    write_pack_with(path, |_| {});
}

/// Replace the schema text of object `name` (a crafted file).
pub fn craft_schema(c: &Connection, name: &str, sql: &str) {
    c.execute_batch("PRAGMA writable_schema = ON")
        .expect("writable_schema");
    c.execute(
        "UPDATE sqlite_schema SET sql = ?1 WHERE name = ?2",
        [sql, name],
    )
    .expect("craft schema");
    c.execute_batch("PRAGMA writable_schema = OFF")
        .expect("writable_schema off");
}

/// Strings meant to attack a terminal: escape sequences (OSC title, clear
/// screen, 8-bit CSI), line breaks forging an `info` line, bidi override.
pub const HOSTILE_NAME: &str =
    "Evil\x1b]0;pwned\x07\x1b[2J\nprofile: resilience-1\r\u{9b}31m\u{202e}txet";

/// The valid pack with hostile strings in its metadata.
pub fn write_hostile_strings_pack(path: &Path) {
    write_pack_with(path, |c| {
        for key in [
            "name",
            "attribution",
            "resilience_coverage",
            "resilience_source",
        ] {
            c.execute(
                "UPDATE metadata SET value = ?1 WHERE name = ?2",
                [HOSTILE_NAME, key],
            )
            .expect("hostile");
        }
    });
}

// ---------------------------------------------------------------------------
// Viewer fixtures (ADR 0016 slice 6): packs with chosen metadata, coverage
// and tiles built from geographic coordinates.
// ---------------------------------------------------------------------------

/// Tile coordinates (extent 4096) of a position in tile `id`. Positions
/// outside the tile give coordinates outside `0..4096` (the buffer).
pub fn tile_xy(id: TileId, lat: f64, lon: f64) -> (i32, i32) {
    let w = project(LatLon::wrapped(lat, lon));
    let n = f64::from(1u32 << id.z());
    let x = (w.x * n - f64::from(id.x())) * 4096.0;
    let y = (w.y * n - f64::from(id.y())) * 4096.0;
    (x.round() as i32, y.round() as i32)
}

/// An attribute value for [`TileMaker`].
#[derive(Debug, Clone, Copy)]
pub enum Attr<'a> {
    S(&'a str),
    I(i64),
}

/// Builds one vector tile from features given in tile or geographic
/// coordinates, interning keys and values per layer.
pub struct TileMaker {
    pub id: TileId,
    layers: Vec<(LayerBuilder, Vec<Vec<u8>>)>,
}

impl TileMaker {
    pub fn new(id: TileId) -> TileMaker {
        TileMaker {
            id,
            layers: Vec::new(),
        }
    }

    fn layer(&mut self, name: &str) -> usize {
        if let Some(i) = self
            .layers
            .iter()
            .position(|(l, _)| l.name.as_deref() == Some(name))
        {
            return i;
        }
        self.layers.push((LayerBuilder::new(name), Vec::new()));
        self.layers.len() - 1
    }

    fn tags(&mut self, li: usize, attrs: &[(&str, Attr<'_>)]) -> Vec<u32> {
        let (l, raw) = &mut self.layers[li];
        let mut tags = Vec::new();
        for (k, v) in attrs {
            let ki = match l.keys.iter().position(|x| x == k) {
                Some(i) => i,
                None => {
                    l.keys.push((*k).to_string());
                    l.keys.len() - 1
                }
            };
            let enc = match v {
                Attr::S(s) => value(&TestValue::Str(s)),
                Attr::I(i) => value(&TestValue::Int(*i)),
            };
            let vi = match raw.iter().position(|x| *x == enc) {
                Some(i) => i,
                None => {
                    raw.push(enc.clone());
                    l.values.push(enc);
                    raw.len() - 1
                }
            };
            tags.push(ki as u32);
            tags.push(vi as u32);
        }
        tags
    }

    /// A feature with raw geometry integers (from the encoder helpers).
    pub fn raw(
        &mut self,
        layer: &str,
        geom_type: u32,
        attrs: &[(&str, Attr<'_>)],
        geometry: &[u32],
    ) {
        let li = self.layer(layer);
        let tags = self.tags(li, attrs);
        self.layers[li]
            .0
            .features
            .push(feature(None, geom_type, &tags, geometry));
    }

    pub fn line_xy(&mut self, layer: &str, attrs: &[(&str, Attr<'_>)], pts: &[(i32, i32)]) {
        self.raw(layer, 2, attrs, &lines(&[pts]));
    }

    pub fn polygon_xy(&mut self, layer: &str, attrs: &[(&str, Attr<'_>)], ring: &[(i32, i32)]) {
        self.raw(layer, 3, attrs, &polygon(&[ring]));
    }

    /// A point at a position (lat, lon).
    pub fn point(&mut self, layer: &str, attrs: &[(&str, Attr<'_>)], at: (f64, f64)) {
        let p = tile_xy(self.id, at.0, at.1);
        self.raw(layer, 1, attrs, &points(&[p]));
    }

    /// A line through positions (lat, lon).
    pub fn line(&mut self, layer: &str, attrs: &[(&str, Attr<'_>)], pts: &[(f64, f64)]) {
        let xy: Vec<(i32, i32)> = pts.iter().map(|&(a, o)| tile_xy(self.id, a, o)).collect();
        self.line_xy(layer, attrs, &xy);
    }

    /// A polygon (one exterior ring) through positions (lat, lon).
    pub fn polygon(&mut self, layer: &str, attrs: &[(&str, Attr<'_>)], ring: &[(f64, f64)]) {
        let xy: Vec<(i32, i32)> = ring.iter().map(|&(a, o)| tile_xy(self.id, a, o)).collect();
        self.polygon_xy(layer, attrs, &xy);
    }

    /// The uncompressed tile.
    pub fn encode(&self) -> Vec<u8> {
        let layers: Vec<Vec<u8>> = self.layers.iter().map(|(l, _)| l.encode()).collect();
        tile(&layers)
    }
}

/// A synthetic pack for the viewer tests: metadata, coverage and tiles of
/// the caller's choice, written with the tippecanoe schema.
#[derive(Debug, Clone)]
pub struct PackSpec {
    pub name: String,
    pub minzoom: u8,
    pub maxzoom: u8,
    /// West, south, east, north; `None` leaves `bounds` out.
    pub bounds: Option<(f64, f64, f64, f64)>,
    /// Lon, lat, zoom; `None` leaves `center` out.
    pub center: Option<(f64, f64, u8)>,
    pub data_date: String,
    pub attribution: Option<String>,
    pub coverage_text: String,
    /// One ring of (lon, lat); `None`: no `rs_coverage` table.
    pub coverage: Option<Vec<(f64, f64)>>,
    /// Uncompressed MVT bytes, gzipped when written.
    pub tiles: Vec<(TileId, Vec<u8>)>,
    /// Stored exactly as given (damaged tiles).
    pub raw_tiles: Vec<(TileId, Vec<u8>)>,
}

impl PackSpec {
    /// A `resilience-1` pack covering the rectangle `(west, south, east,
    /// north)` with that rectangle as its coverage polygon, zooms 0-14.
    pub fn detailed(name: &str, b: (f64, f64, f64, f64)) -> PackSpec {
        PackSpec {
            name: name.to_string(),
            minzoom: 0,
            maxzoom: 14,
            bounds: Some(b),
            center: None,
            data_date: "2026-01-01".into(),
            attribution: Some("© OpenStreetMap contributors".into()),
            coverage_text: format!("{name}, whole region"),
            coverage: Some(vec![(b.0, b.1), (b.2, b.1), (b.2, b.3), (b.0, b.3)]),
            tiles: Vec::new(),
            raw_tiles: Vec::new(),
        }
    }

    /// An overview pack (zooms 0-7, whole world, no coverage polygon).
    pub fn overview(name: &str) -> PackSpec {
        PackSpec {
            name: name.to_string(),
            minzoom: 0,
            maxzoom: 7,
            bounds: None,
            center: None,
            data_date: "2022-05-01".into(),
            attribution: Some("Natural Earth".into()),
            coverage_text: "The whole world at 1:10 million; no streets".into(),
            coverage: None,
            tiles: Vec::new(),
            raw_tiles: Vec::new(),
        }
    }

    pub fn tile(mut self, t: TileMaker) -> PackSpec {
        let bytes = t.encode();
        self.tiles.push((t.id, bytes));
        self
    }

    pub fn write(&self, path: &Path) {
        let c = Connection::open(path).expect("create pack");
        c.execute_batch(TIPPECANOE_SCHEMA).expect("schema");
        c.execute_batch("BEGIN").expect("begin");
        let mut meta: Vec<(String, String)> = vec![
            ("name".into(), self.name.clone()),
            ("format".into(), "pbf".into()),
            ("minzoom".into(), self.minzoom.to_string()),
            ("maxzoom".into(), self.maxzoom.to_string()),
            ("resilience_profile".into(), "1".into()),
            ("resilience_data_date".into(), self.data_date.clone()),
            (
                "resilience_source".into(),
                format!("{}.osm.pbf sha256:0123", self.name.to_lowercase()),
            ),
            ("resilience_coverage".into(), self.coverage_text.clone()),
            ("resilience_license".into(), "ODbL-1.0".into()),
        ];
        if let Some(a) = &self.attribution {
            meta.push(("attribution".into(), a.clone()));
        }
        if let Some((w, s, e, n)) = self.bounds {
            meta.push(("bounds".into(), format!("{w},{s},{e},{n}")));
        }
        if let Some((lon, lat, z)) = self.center {
            meta.push(("center".into(), format!("{lon},{lat},{z}")));
        }
        for (k, v) in &meta {
            c.execute("INSERT INTO metadata (name, value) VALUES (?1, ?2)", [k, v])
                .expect("metadata");
        }
        let put = |id: TileId, blob: Vec<u8>| {
            let t = id.to_tms();
            c.execute(
                "INSERT INTO tiles VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![t.zoom(), t.column(), t.row(), blob],
            )
            .expect("tile");
        };
        for (id, bytes) in &self.tiles {
            put(*id, gzip(bytes));
        }
        for (id, bytes) in &self.raw_tiles {
            put(*id, bytes.clone());
        }
        if let Some(ring) = &self.coverage {
            c.execute_batch(profile::RS_COVERAGE).expect("rs_coverage");
            let ring: Vec<Micro> = ring
                .iter()
                .map(|&(lon, lat)| Micro::from_degrees(lon, lat).expect("in the world"))
                .collect();
            c.execute(
                "INSERT INTO rs_coverage (ring, hole, points) VALUES (1, 0, ?1)",
                [profile::encode_ring(&ring)],
            )
            .expect("coverage");
        }
        c.execute_batch("COMMIT").expect("commit");
        c.close().expect("close pack");
    }
}
