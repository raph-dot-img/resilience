//! Synthetic map packs for tests, written at test time into temporary
//! directories (no binary files in Git). Shared by the unit tests and by
//! `tests/cli.rs`, which includes this file with `#[path]`; it therefore
//! names the library as `resilience_maps` (see `lib.rs`).

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use resilience_core::maps::geo::TmsTile;
use resilience_core::maps::mvt_encode::{
    LayerBuilder, TestValue, feature, lines, points, tile, value,
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
