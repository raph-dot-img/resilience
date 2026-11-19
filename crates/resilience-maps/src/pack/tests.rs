//! Tests of the hardened reader on synthetic packs (ADR 0016 §8). Every
//! test that opens a file checks that nothing was written beside it.

use std::time::{Duration, Instant};

use resilience_core::maps::geo::{TileId, TmsTile};

use super::*;
use crate::fixtures::*;
use crate::metadata::MetadataError;

/// Run `f`, asserting it returns within `limit` (debug builds are slow;
/// the bounds that matter are far below these).
fn within<T>(limit: Duration, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    assert!(t.elapsed() < limit, "took {:?}", t.elapsed());
    out
}

const PROMPT: Duration = Duration::from_secs(3);

/// Open `path` checking promptness and that its directory is unchanged.
fn open(path: &Path) -> Result<Pack, PackError> {
    let dir = path.parent().expect("parent");
    unchanged(dir, || within(PROMPT, || Pack::open(path)))
}

/// Build a pack with `alter`, open it, return the result.
fn open_altered(tag: &str, alter: impl FnOnce(&Connection)) -> (TempDir, Result<Pack, PackError>) {
    let dir = TempDir::new(tag);
    let path = dir.file("map.mbtiles");
    write_pack_with(&path, alter);
    let r = open(&path);
    (dir, r)
}

fn label_of(tile: &mvt::Tile) -> String {
    let places = tile.layer("places").expect("places layer");
    let f = places.feature(0).expect("one feature");
    f.attr("name")
        .and_then(|v| v.as_str())
        .expect("name")
        .to_string()
}

#[test]
fn a_valid_pack_reads_back() {
    let dir = TempDir::new("valid");
    let path = dir.file("map.mbtiles");
    write_pack(&path);
    let pack = open(&path).expect("open");
    unchanged(dir.path(), || {
        let m = pack.metadata();
        assert_eq!(m.name, "Testland");
        assert_eq!(m.profile, Profile::Resilience1);
        assert_eq!((m.zoom.min(), m.zoom.max()), (0, 2));
        assert!(m.bounds_given);
        assert_eq!(m.bounds.west, 5.7);
        assert_eq!(m.data_date.as_deref(), Some("2026-01-01"));
        assert_eq!(m.license.as_deref(), Some("ODbL-1.0"));
        assert_eq!(
            m.attribution.as_deref(),
            Some("(c) OpenStreetMap contributors")
        );
        assert_eq!(m.coverage.as_deref(), Some("Testland, whole country"));
        assert!(m.source.as_deref().is_some_and(|s| s.contains("sha256:")));
        let c = m.center.expect("center");
        assert!((c.at.lat() - 49.6116).abs() < 1e-9 && c.zoom == Some(2));
        assert_eq!(pack.layout(), TileLayout::Table);
        assert_eq!(pack.path(), path.as_path());
        assert!(pack.quick_check_recommended());

        // Tiles through both numbering conventions.
        for (tms, label) in sample_tiles() {
            let xyz: TileId = tms.to_xyz();
            let a = within(Duration::from_secs(1), || pack.tile(tms))
                .expect("tile")
                .expect("present");
            let b = pack.tile(xyz).expect("tile").expect("present");
            assert_eq!(label_of(&a), label);
            assert_eq!(label_of(&b), label);
            assert_eq!(
                pack.tile_bytes(tms).expect("should succeed"),
                Some(sample_tile(&label))
            );
        }
        // XYZ row 0 is the north: TMS (1, 0, 0) is XYZ (1, 0, 1).
        let north_west = TileId::new(1, 0, 1).expect("should succeed");
        assert_eq!(
            label_of(
                &pack
                    .tile(north_west)
                    .expect("should succeed")
                    .expect("should succeed")
            ),
            "T1-0-0"
        );
        assert!(
            pack.tile(TmsTile::new(1, 1, 0).expect("should succeed"))
                .expect("should succeed")
                .is_none()
        );
        assert!(
            pack.tile(TmsTile::new(3, 0, 0).expect("should succeed"))
                .expect("should succeed")
                .is_none()
        );

        let caps = pack.capabilities();
        assert_eq!(caps.coverage, Ok(()));
        assert_eq!(caps.search, Ok(()));
        assert_eq!(caps.nearby, Ok(()));
        let cov = pack.coverage().expect("should succeed").expect("coverage");
        assert_eq!(
            cov.rings,
            vec![Ring {
                hole: false,
                points: sample_ring()
            }]
        );
        assert_eq!(cov.vertex_count(), 4);
        assert_eq!(pack.tile_count(), Ok(Some(4)));
        assert_eq!(pack.quick_check(), Ok(()));
    });
}

#[test]
fn search_and_nearby_tables_work_under_the_hardening() {
    // Slices 10 and 11 add the queries; this proves the authorizer and
    // limits let FTS5 and R*Tree work on a verified file.
    let dir = TempDir::new("fts");
    let path = dir.file("map.mbtiles");
    write_pack(&path);
    let pack = open(&path).expect("should succeed");
    unchanged(dir.path(), || {
        let name: String = pack
            .conn
            .query_row(
                "SELECT p.name FROM rs_place_fts f JOIN rs_place p ON p.id = f.rowid \
                 WHERE rs_place_fts MATCH ?1 ORDER BY f.rank LIMIT 50",
                ["\"essa\"*"],
                |r| r.get(0),
            )
            .expect("should succeed");
        assert_eq!(name, "Rue de l'Essai");
        let id: i64 = pack
            .conn
            .query_row(
                "SELECT id FROM rs_poi_rtree WHERE min_lon <= ?1 AND max_lon >= ?2 \
                 AND min_lat <= ?3 AND max_lat >= ?4",
                [6_140_000, 6_120_000, 49_620_000, 49_600_000],
                |r| r.get(0),
            )
            .expect("should succeed");
        assert_eq!(id, 2);
    });
}

#[test]
fn random_bytes_are_not_a_map() {
    let dir = TempDir::new("random");
    let path = dir.file("random.mbtiles");
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let bytes: Vec<u8> = (0..65_536)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect();
    std::fs::write(&path, &bytes).expect("should succeed");
    assert_eq!(open(&path).expect_err("should fail"), PackError::NotSqlite);
    std::fs::write(&path, b"").expect("should succeed");
    assert_eq!(open(&path).expect_err("should fail"), PackError::NotSqlite);
}

#[test]
fn a_truncated_file_is_refused() {
    let dir = TempDir::new("truncated");
    let full = dir.file("full.mbtiles");
    write_pack(&full);
    let bytes = std::fs::read(&full).expect("should succeed");
    let page = 4096;
    assert_eq!(bytes.len() % page, 0);
    let cut = dir.file("cut.mbtiles");
    for len in [bytes.len() / 2 + 100, bytes.len() - page, page, 50, 10] {
        std::fs::write(&cut, &bytes[..len]).expect("should succeed");
        assert_eq!(
            open(&cut).expect_err("should fail"),
            PackError::Truncated,
            "length {len}"
        );
    }
}

#[test]
fn a_symlink_is_refused() {
    let dir = TempDir::new("symlink");
    let real = dir.file("real.mbtiles");
    write_pack(&real);
    let link = dir.file("link.mbtiles");
    std::os::unix::fs::symlink(&real, &link).expect("should succeed");
    assert_eq!(open(&link).expect_err("should fail"), PackError::Symlink);
    // SQLite's own NOFOLLOW refuses it too, should our check be bypassed.
    assert_eq!(
        open_connection(&link).expect_err("should fail"),
        PackError::Symlink
    );
    // A symlink in the directory part is resolved and accepted.
    let sub = dir.file("sub");
    std::os::unix::fs::symlink(dir.path(), &sub).expect("should succeed");
    let pack = open(&sub.join("real.mbtiles")).expect("via a directory link");
    assert_eq!(pack.path(), real.as_path());
}

#[test]
fn directories_and_fifos_are_refused() {
    let dir = TempDir::new("notfile");
    let sub = dir.file("dir.mbtiles");
    std::fs::create_dir(&sub).expect("should succeed");
    assert_eq!(open(&sub).expect_err("should fail"), PackError::NotAFile);
    let fifo = dir.file("fifo.mbtiles");
    let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).expect("should succeed");
    // SAFETY: mkfifo reads a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    // O_NONBLOCK: opening a FIFO without a writer does not block.
    assert_eq!(open(&fifo).expect_err("should fail"), PackError::NotAFile);
    assert!(matches!(
        open(&dir.file("missing.mbtiles")),
        Err(PackError::Io(_))
    ));
}

#[test]
fn a_file_in_wal_mode_is_refused() {
    let (dir, r) = open_altered("wal", |c| {
        let mode: String = c
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .expect("should succeed");
        assert_eq!(mode, "wal");
    });
    // Closing the builder's connection removed its -wal and -shm files.
    assert_eq!(listing(dir.path()).len(), 1);
    assert_eq!(r.expect_err("should fail"), PackError::WalMode);
}

#[test]
fn a_recursive_view_as_tiles_is_refused_and_cannot_run() {
    let (dir, r) = open_altered("recursive", |c| {
        c.execute_batch(
            "DROP TABLE tiles;
             CREATE VIEW tiles AS WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r)
             SELECT n AS zoom_level, n AS tile_column, n AS tile_row, randomblob(1000) AS tile_data FROM r;",
        )
        .expect("should succeed");
    });
    assert_eq!(
        r.expect_err("should fail"),
        PackError::Schema(SchemaError::NotATable("tiles"))
    );
    // Even if a statement named it, the view could not run: views are
    // disabled on the connection and the authorizer refuses recursion.
    let path = dir.file("map.mbtiles");
    let mut conn = open_connection(&path).expect("should succeed");
    harden(&mut conn).expect("should succeed");
    let r = within(PROMPT, || {
        conn.query_row("SELECT count(*) FROM tiles", [], |r| r.get::<_, i64>(0))
    });
    assert!(r.is_err(), "{r:?}");
    let e = conn
        .query_row(
            "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r) SELECT count(*) FROM r",
            [],
            |r| r.get::<_, i64>(0),
        )
        .expect_err("should fail");
    assert!(matches!(sql_err(e), PackError::Denied(_)));
}

#[test]
fn deduplicated_layouts_are_read_through_their_tables_never_the_view() {
    // mbutil / tilemaker: tiles is a view over map and images. The view
    // here never ends; the reader must not care.
    let (_dir, r) = open_altered("mapimages", |c| {
        c.execute_batch(
            "CREATE TABLE map (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_id TEXT, grid_id TEXT);
             CREATE TABLE images (tile_data BLOB, tile_id TEXT);
             CREATE UNIQUE INDEX map_index ON map (zoom_level, tile_column, tile_row);
             CREATE UNIQUE INDEX images_id ON images (tile_id);
             INSERT INTO map SELECT zoom_level, tile_column, tile_row, 'h' || rowid, NULL FROM tiles;
             INSERT INTO images SELECT tile_data, 'h' || rowid FROM tiles;
             DROP TABLE tiles;
             CREATE VIEW tiles AS WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r)
             SELECT n AS zoom_level, n AS tile_column, n AS tile_row, n AS tile_data FROM r;",
        )
        .expect("should succeed");
    });
    let pack = r.expect("map/images layout");
    assert_eq!(pack.layout(), TileLayout::MapImages);
    for (t, label) in sample_tiles() {
        let tile = within(Duration::from_secs(1), || pack.tile(t))
            .expect("should succeed")
            .expect("should succeed");
        assert_eq!(label_of(&tile), label);
    }
    assert_eq!(pack.tile_count(), Ok(Some(4)));

    // Planetiler's compact layout.
    let (_dir, r) = open_altered("shallow", |c| {
        c.execute_batch(
            "CREATE TABLE tiles_shallow (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data_id INTEGER,
                 PRIMARY KEY (zoom_level, tile_column, tile_row)) WITHOUT ROWID;
             CREATE TABLE tiles_data (tile_data_id INTEGER PRIMARY KEY, tile_data BLOB);
             INSERT INTO tiles_shallow SELECT zoom_level, tile_column, tile_row, rowid FROM tiles;
             INSERT INTO tiles_data SELECT rowid, tile_data FROM tiles;
             DROP TABLE tiles;
             CREATE VIEW tiles AS SELECT zoom_level, tile_column, tile_row, randomblob(100000000) AS tile_data
                 FROM tiles_shallow;",
        )
        .expect("should succeed");
    });
    let pack = r.expect("planetiler layout");
    assert_eq!(pack.layout(), TileLayout::ShallowData);
    let (t, label) = &sample_tiles()[2];
    assert_eq!(
        label_of(
            &pack
                .tile(*t)
                .expect("should succeed")
                .expect("should succeed")
        ),
        *label
    );
}

#[test]
fn metadata_as_a_view_calling_an_expensive_function_is_refused() {
    let (_dir, r) = open_altered("metaview", |c| {
        c.execute_batch(
            "ALTER TABLE metadata RENAME TO meta2;
             CREATE VIEW metadata AS SELECT name, value FROM meta2 WHERE length(randomblob(1000000000)) > 0;",
        )
        .expect("should succeed");
    });
    assert_eq!(
        r.expect_err("should fail"),
        PackError::Schema(SchemaError::NotATable("metadata"))
    );
}

#[test]
fn load_extension_and_attach_are_impossible() {
    // A view trying to load a library, in place of tiles.
    let (dir, r) = open_altered("loadext", |c| {
        c.execute_batch(
            "DROP TABLE tiles;
             CREATE VIEW tiles AS SELECT 0 AS zoom_level, 0 AS tile_column, 0 AS tile_row,
                 load_extension('/tmp/evil.so') AS tile_data;",
        )
        .expect("should succeed");
    });
    assert_eq!(
        r.expect_err("should fail"),
        PackError::Schema(SchemaError::NotATable("tiles"))
    );

    // Directly on a hardened connection.
    let path = dir.file("map.mbtiles");
    let mut conn = open_connection(&path).expect("should succeed");
    harden(&mut conn).expect("should succeed");
    for sql in [
        "SELECT load_extension('/tmp/evil.so')",
        "ATTACH DATABASE '/tmp/other.db' AS other",
        "ATTACH DATABASE ':memory:' AS other",
        "SELECT readfile('/etc/passwd')",
        "PRAGMA query_only = 0",
        "PRAGMA writable_schema = 1",
        "INSERT INTO metadata VALUES ('a', 'b')",
        "SELECT * FROM rs_place",
    ] {
        assert!(conn.execute_batch(sql).is_err(), "{sql}");
    }
    assert!(!dir.file("other.db").exists());
}

#[test]
fn sqlite_is_built_without_extension_loading() {
    let conn = Connection::open_in_memory().expect("should succeed");
    let mut st = conn
        .prepare("PRAGMA compile_options")
        .expect("should succeed");
    let options: Vec<String> = st
        .query_map([], |r| r.get(0))
        .expect("should succeed")
        .collect::<Result<_, _>>()
        .expect("should succeed");
    assert!(
        options.iter().any(|o| o == "OMIT_LOAD_EXTENSION"),
        "{options:?}"
    );
    assert!(
        options.iter().any(|o| o == "MAX_MMAP_SIZE=0"),
        "{options:?}"
    );
    assert!(
        !options
            .iter()
            .any(|o| o.starts_with("ENABLE_LOAD_EXTENSION"))
    );
    let e = conn
        .query_row("SELECT load_extension('x')", [], |r| r.get::<_, i64>(0))
        .expect_err("should fail");
    assert!(e.to_string().contains("no such function"), "{e}");
}

#[test]
fn a_table_with_a_trigger_is_refused() {
    let (_dir, r) = open_altered("trigger", |c| {
        c.execute_batch(
            "CREATE TRIGGER t AFTER INSERT ON metadata BEGIN SELECT randomblob(1000000000); END;",
        )
        .expect("should succeed");
    });
    assert_eq!(
        r.expect_err("should fail"),
        PackError::Schema(SchemaError::Trigger)
    );
}

#[test]
fn computed_columns_and_indexes_are_refused() {
    let tiles = "DROP TABLE tiles; CREATE TABLE tiles (zoom_level integer, tile_column integer, tile_row integer";
    for (tag, sql, err) in [
        (
            "generated",
            format!("{tiles}, raw blob, tile_data blob GENERATED ALWAYS AS (zeroblob(4000000)) VIRTUAL);"),
            SchemaError::ComputedColumn("tiles"),
        ),
        (
            "default",
            format!("{tiles}, tile_data blob DEFAULT (zeroblob(4000000)));"),
            SchemaError::ComputedColumn("tiles"),
        ),
        (
            "column",
            format!("{tiles});"),
            SchemaError::MissingColumn("tiles", "tile_data"),
        ),
        (
            "expression-index",
            "CREATE INDEX e ON tiles (abs(zoom_level));".to_string(),
            SchemaError::ComputedIndex("tiles"),
        ),
        (
            "partial-index",
            "CREATE INDEX p ON metadata (name) WHERE value IS NOT NULL;".to_string(),
            SchemaError::ComputedIndex("metadata"),
        ),
        (
            "virtual",
            "DROP TABLE tiles; CREATE VIRTUAL TABLE tiles USING fts5(zoom_level, tile_column, tile_row, tile_data);"
                .to_string(),
            SchemaError::NotATable("tiles"),
        ),
    ] {
        let (_dir, r) = open_altered(tag, |c| c.execute_batch(&sql).expect("should succeed"));
        assert_eq!(r.expect_err("should fail"), PackError::Schema(err), "{tag}");
    }
}

#[test]
fn crafted_schema_text_is_refused() {
    // A collation the reader does not have: our statements cannot compile.
    let (_dir, r) = open_altered("collation", |c| {
        craft_schema(
            c,
            "tiles",
            "CREATE TABLE tiles (zoom_level integer COLLATE evil, tile_column integer, tile_row integer, tile_data blob)",
        );
    });
    assert_eq!(
        r.expect_err("should fail"),
        PackError::Schema(SchemaError::Unusable(
            "no such collation sequence: evil".into()
        ))
    );
    // A row whose type disagrees with its text.
    let (_dir, r) = open_altered("liar", |c| {
        c.execute_batch("CREATE TABLE innocent (a)")
            .expect("should succeed");
        c.execute_batch("PRAGMA writable_schema = ON")
            .expect("should succeed");
        c.execute(
            "UPDATE sqlite_schema SET type = 'index' WHERE name = 'innocent'",
            [],
        )
        .expect("should succeed");
        c.execute_batch("PRAGMA writable_schema = OFF")
            .expect("should succeed");
    });
    // SQLite itself notices this one while loading the schema.
    assert!(matches!(r, Err(PackError::Corrupt(_))), "{r:?}");
}

#[test]
fn an_rs_table_differing_by_one_character_disables_only_its_feature() {
    let dir = TempDir::new("onechar");
    let path = dir.file("map.mbtiles");
    let altered = profile::RS_PLACE_FTS.replace("prefix='2 3'", "prefix='2 4'");
    assert_eq!(altered.len(), profile::RS_PLACE_FTS.len());
    write_pack_with(&path, |c| {
        c.execute_batch(
            "DROP TABLE rs_place_fts; DROP TABLE rs_place; DROP TABLE rs_coverage; DROP TABLE rs_poi_rtree;",
        )
        .expect("should succeed");
        write_rs_tables(
            c,
            &[
                profile::RS_COVERAGE,
                profile::RS_PLACE,
                &altered,
                profile::RS_POI_RTREE,
            ],
        );
    });
    let pack = open(&path).expect("the map still opens");
    let caps = pack.capabilities();
    assert_eq!(
        caps.search,
        Err(Unavailable::Differs("rs_place_fts".into()))
    );
    assert_eq!(caps.nearby, Ok(()));
    assert_eq!(caps.coverage, Ok(()));
    assert!(
        pack.tile(sample_tiles()[0].0)
            .expect("should succeed")
            .is_some()
    );
    // The authorizer does not let a statement read the unverified table.
    let e = pack
        .conn
        .query_row("SELECT count(*) FROM rs_place_fts_data", [], |r| {
            r.get::<_, i64>(0)
        })
        .expect_err("should fail");
    assert!(matches!(sql_err(e), PackError::Denied(_)));

    // A third-party file (no profile) never gets the features.
    let (_dir, r) = open_altered("noprofile", |c| {
        c.execute("DELETE FROM metadata WHERE name = 'resilience_profile'", [])
            .expect("should succeed");
    });
    let pack = r.expect("should succeed");
    assert_eq!(pack.capabilities().search, Err(Unavailable::NotOurProfile));
    assert_eq!(pack.coverage(), Ok(None));
    // And a pack without rs_ tables says they are absent.
    let (_dir, r) = open_altered("nors", |c| {
        c.execute_batch("DROP TABLE rs_coverage;")
            .expect("should succeed");
    });
    assert_eq!(
        r.expect("should succeed").capabilities().coverage,
        Err(Unavailable::Absent)
    );
}

fn pack_with_tile(tag: &str, blob: Vec<u8>) -> (TempDir, Pack, TmsTile) {
    let t = TmsTile::new(2, 3, 3).expect("should succeed");
    let (dir, r) = open_altered(tag, |c| {
        c.execute(
            "INSERT INTO tiles VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![t.zoom(), t.column(), t.row(), blob],
        )
        .expect("should succeed");
    });
    (dir, r.expect("opens: tiles are read on demand"), t)
}

#[test]
fn a_gzip_bomb_tile_is_refused_promptly() {
    let bomb = crate::inflate::gzip(&vec![0u8; 64 * 1024 * 1024]);
    assert!(bomb.len() < 256 * 1024);
    let (dir, pack, t) = pack_with_tile("bomb", bomb);
    let r = unchanged(dir.path(), || within(PROMPT, || pack.tile(t)));
    assert_eq!(
        r.expect_err("should fail"),
        PackError::Tile(TileError::TooLargeDecompressed)
    );
    // The other tiles are still fine.
    assert!(
        pack.tile(sample_tiles()[0].0)
            .expect("should succeed")
            .is_some()
    );
}

#[test]
fn a_tile_over_the_compressed_limit_is_refused() {
    let mut blob = vec![0x1f, 0x8b];
    blob.resize(MAX_COMPRESSED + 1, 0x55);
    let (dir, pack, t) = pack_with_tile("huge", blob);
    let r = unchanged(dir.path(), || within(PROMPT, || pack.tile(t)));
    assert_eq!(
        r.expect_err("should fail"),
        PackError::Tile(TileError::TooLargeCompressed)
    );
}

#[test]
fn a_tile_that_is_not_a_vector_tile_is_refused() {
    let (_dir, pack, t) = pack_with_tile("png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec());
    assert_eq!(
        pack.tile(t).expect_err("should fail"),
        PackError::Tile(TileError::UnknownEncoding)
    );
    let (_dir, pack, t) =
        pack_with_tile("badmvt", crate::inflate::gzip(b"\x1a\xff\xff\xff\xff\x0f"));
    assert!(matches!(
        pack.tile(t),
        Err(PackError::Tile(TileError::Mvt(_)))
    ));
}

#[test]
fn missing_or_absurd_metadata_is_refused() {
    use MetadataError::*;
    for (tag, sql, err) in [
        (
            "nomax",
            "DELETE FROM metadata WHERE name = 'maxzoom'",
            Missing("maxzoom"),
        ),
        (
            "noname",
            "DELETE FROM metadata WHERE name = 'name'",
            Missing("name"),
        ),
        (
            "reversed",
            "UPDATE metadata SET value = '15' WHERE name = 'minzoom'",
            Invalid("zoom range", "minzoom > maxzoom"),
        ),
        (
            "bounds",
            "UPDATE metadata SET value = '-200,-95,200,95' WHERE name = 'bounds'",
            Invalid("bounds", "outside the world"),
        ),
        (
            "bigname",
            "UPDATE metadata SET value = printf('%.*c', 1048576, 'x') WHERE name = 'name'",
            TooLong("name"),
        ),
        (
            "raster",
            "UPDATE metadata SET value = 'png' WHERE name = 'format'",
            UnsupportedFormat("png".into()),
        ),
        (
            "date",
            "UPDATE metadata SET value = 'last week' WHERE name = 'resilience_data_date'",
            Invalid("resilience_data_date", "not YYYY-MM-DD or YYYY-MM"),
        ),
        (
            "nolicense",
            "DELETE FROM metadata WHERE name = 'resilience_license'",
            Missing("resilience_license"),
        ),
    ] {
        let (_dir, r) = open_altered(tag, |c| {
            c.execute_batch(sql).expect("should succeed");
        });
        assert_eq!(
            r.expect_err("should fail"),
            PackError::Metadata(err),
            "{tag}"
        );
    }
}

#[test]
fn hostile_strings_are_cleaned() {
    let dir = TempDir::new("strings");
    let path = dir.file("map.mbtiles");
    write_hostile_strings_pack(&path);
    let pack = open(&path).expect("should succeed");
    let m = pack.metadata();
    for s in [
        &m.name,
        m.attribution.as_ref().expect("should succeed"),
        m.coverage.as_ref().expect("should succeed"),
    ] {
        assert!(!s.chars().any(crate::text::is_unsafe), "{s:?}");
        assert!(s.starts_with("Evil?]0;pwned"), "{s:?}");
    }
}

#[test]
fn the_progress_handler_stops_long_statements() {
    // A big table without an index: every lookup is a full scan.
    let dir = TempDir::new("slow");
    let path = dir.file("map.mbtiles");
    write_pack_with(&path, |c| {
        c.execute_batch("DROP INDEX tile_index; BEGIN;")
            .expect("should succeed");
        let mut st = c
            .prepare("INSERT INTO tiles VALUES (14, ?1, ?2, x'1a00')")
            .expect("should succeed");
        for i in 0..60_000u32 {
            st.execute([i % 16_384, i / 16_384])
                .expect("should succeed");
        }
        drop(st);
        c.execute_batch("COMMIT;").expect("should succeed");
    });
    let pack = open(&path).expect("should succeed");
    let missing = TmsTile::new(2, 3, 3).expect("should succeed");
    // With a budget far below what a scan of 60,000 rows needs.
    let small = Budget {
        time: Duration::from_secs(10),
        ops: 100_000,
    };
    let r = within(PROMPT, || pack.tile_bytes_within(missing, small));
    assert_eq!(r, Err(PackError::TooSlow));
    // The real tile budget still allows this scan (about 500,000 steps).
    assert_eq!(pack.tile_bytes(missing), Ok(None));
    // A statement that would run for hours stops at QUERY_BUDGET.
    pack.budget.arm(QUERY_BUDGET);
    let e = within(Duration::from_secs(6), || {
        pack.conn
            .query_row("SELECT count(*) FROM tiles a, tiles b, tiles c", [], |r| {
                r.get::<_, i64>(0)
            })
    })
    .expect_err("should fail");
    assert_eq!(sql_err(e), PackError::TooSlow);
    // quick_check is bounded the same way.
    let tiny = Budget {
        time: Duration::from_secs(10),
        ops: 1_000,
    };
    assert_eq!(pack.quick_check_within(tiny), Err(PackError::TooSlow));
    assert_eq!(pack.quick_check(), Ok(()));
}

#[test]
fn a_huge_schema_is_refused() {
    let (_dir, r) = open_altered("manyobjects", |c| {
        let mut sql = String::new();
        for i in 0..300 {
            sql.push_str(&format!("CREATE TABLE extra{i} (a);"));
        }
        c.execute_batch(&sql).expect("should succeed");
    });
    assert_eq!(
        r.expect_err("should fail"),
        PackError::Schema(SchemaError::TooManyObjects)
    );
    // Up to the limit, unknown tables are tolerated.
    let (_dir, r) = open_altered("someobjects", |c| {
        c.execute_batch("CREATE TABLE extra (a); CREATE VIEW v AS SELECT 1; CREATE VIRTUAL TABLE vt USING fts5(x);")
            .expect("should succeed");
    });
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn quick_check_reports_damage() {
    let dir = TempDir::new("damaged");
    let path = dir.file("map.mbtiles");
    write_pack(&path);
    let mut bytes = std::fs::read(&path).expect("should succeed");
    // Find the root page of the tile index and damage its cell content.
    let root: i64 = Connection::open(&path)
        .expect("should succeed")
        .query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name = 'tile_index'",
            [],
            |r| r.get(0),
        )
        .expect("should succeed");
    let start = (root as usize - 1) * 4096;
    for b in &mut bytes[start + 8..start + 4096] {
        *b = 0xa5;
    }
    std::fs::write(&path, &bytes).expect("should succeed");
    // The schema is intact, so the file opens; quick_check finds the damage.
    let pack = open(&path).expect("opens");
    match pack.quick_check() {
        Err(PackError::Damaged(m)) => {
            assert!(m.contains("out of range"), "{m}");
            assert!(!m.chars().any(crate::text::is_unsafe), "{m:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_read_only_directory_stays_unchanged() {
    let dir = TempDir::new("readonly");
    let path = dir.file("map.mbtiles");
    write_pack(&path);
    dir.set_read_only(true);
    let pack = open(&path).expect("opens in a read-only directory");
    unchanged(dir.path(), || {
        for (t, _) in sample_tiles() {
            assert!(pack.tile(t).expect("should succeed").is_some());
        }
        assert!(pack.coverage().expect("should succeed").is_some());
        assert_eq!(pack.quick_check(), Ok(()));
        assert_eq!(pack.tile_count(), Ok(Some(4)));
    });
    drop(pack);
    dir.set_read_only(false);
}

#[test]
fn mutated_packs_never_panic_or_hang() {
    let dir = TempDir::new("mutated");
    let good = dir.file("good.mbtiles");
    write_pack(&good);
    let bytes = std::fs::read(&good).expect("should succeed");
    let path = dir.file("bad.mbtiles");
    let mut seed = 0xdead_beef_cafe_f00du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..60 {
        let mut b = bytes.clone();
        for _ in 0..(next() % 40 + 1) {
            // Mostly beyond the header, which is checked separately.
            let i = 100 + (next() as usize) % (b.len() - 100);
            b[i] = next() as u8;
        }
        std::fs::write(&path, &b).expect("should succeed");
        if let Ok(pack) = open(&path) {
            within(PROMPT, || {
                for (t, _) in sample_tiles() {
                    let _ = pack.tile(t);
                }
                let _ = pack.coverage();
                let _ = pack.tile_count();
                let _ = pack.quick_check();
            });
        }
    }
}

#[test]
fn the_reader_works_under_the_address_space_limit() {
    const CHILD: &str = "RESILIENCE_MAPS_RLIMIT_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        // The child: lower the limit, then read the pack.
        crate::rlimit::limit_address_space(crate::rlimit::ADDRESS_SPACE_LIMIT)
            .expect("should succeed");
        assert!(
            crate::rlimit::address_space_limit().expect("should succeed")
                <= crate::rlimit::ADDRESS_SPACE_LIMIT
        );
        // The limit is real: 2 GiB cannot be reserved.
        assert!(Vec::<u8>::new().try_reserve(2 << 30).is_err());
        let pack = Pack::open(Path::new(&path)).expect("should succeed");
        for (t, label) in sample_tiles() {
            assert_eq!(
                label_of(
                    &pack
                        .tile(t)
                        .expect("should succeed")
                        .expect("should succeed")
                ),
                label
            );
        }
        assert!(pack.coverage().expect("should succeed").is_some());
        assert_eq!(pack.quick_check(), Ok(()));
        // 8 MiB decompression buffers fit.
        let big = crate::inflate::gzip(&vec![0u8; crate::inflate::MAX_DECOMPRESSED]);
        assert_eq!(
            crate::inflate::decompress_tile(&big).map(|v| v.len()),
            Ok(crate::inflate::MAX_DECOMPRESSED)
        );
        return;
    }
    let dir = TempDir::new("rlimit");
    let path = dir.file("map.mbtiles");
    write_pack(&path);
    let out = std::process::Command::new(std::env::current_exe().expect("should succeed"))
        .args([
            "--exact",
            "pack::tests::the_reader_works_under_the_address_space_limit",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(CHILD, &path)
        .output()
        .expect("should succeed");
    let log = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{log}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(log.contains("1 passed"), "{log}");
}

#[test]
fn headers_are_checked_before_sqlite_sees_the_file() {
    let mut h = [0u8; 100];
    h[..16].copy_from_slice(MAGIC);
    h[16..18].copy_from_slice(&4096u16.to_be_bytes());
    h[18] = 1;
    h[19] = 1;
    h[21..24].copy_from_slice(&[64, 32, 32]);
    assert_eq!(check_header(&h, 8192), Ok(()));
    assert_eq!(check_header(&h, 8191), Err(PackError::Truncated));
    assert_eq!(check_header(&h, 0), Err(PackError::Truncated));
    let mut wal = h;
    wal[18] = 2;
    wal[19] = 2;
    assert_eq!(check_header(&wal, 8192), Err(PackError::WalMode));
    wal[18] = 1;
    assert_eq!(check_header(&wal, 8192), Err(PackError::WalMode));
    let mut v = h;
    v[18] = 3;
    assert_eq!(
        check_header(&v, 8192),
        Err(PackError::BadHeader("file format version"))
    );
    for size in [0u16, 100, 511, 3000] {
        let mut p = h;
        p[16..18].copy_from_slice(&size.to_be_bytes());
        assert_eq!(
            check_header(&p, 8192),
            Err(PackError::BadHeader("page size")),
            "{size}"
        );
    }
    let mut p = h;
    p[16..18].copy_from_slice(&1u16.to_be_bytes());
    assert_eq!(check_header(&p, 65_536), Ok(()));
    let mut f = h;
    f[21] = 65;
    assert_eq!(
        check_header(&f, 8192),
        Err(PackError::BadHeader("payload fractions"))
    );
    // Header page count, valid when bytes 92-95 match the change counter.
    let mut n = h;
    n[28..32].copy_from_slice(&3u32.to_be_bytes());
    n[24..28].copy_from_slice(&7u32.to_be_bytes());
    assert_eq!(check_header(&n, 8192), Ok(()), "count not valid yet");
    n[92..96].copy_from_slice(&7u32.to_be_bytes());
    assert_eq!(check_header(&n, 8192), Err(PackError::Truncated));
    assert_eq!(check_header(&n, 12288), Ok(()));
    assert_eq!(check_header(&h[..40], 40), Err(PackError::Truncated));
    assert_eq!(check_header(b"PK\x03\x04", 4), Err(PackError::NotSqlite));
    let mut m = h;
    m[0] = b's';
    assert_eq!(check_header(&m, 8192), Err(PackError::NotSqlite));
}

#[test]
fn uris_encode_every_unusual_byte() {
    assert_eq!(
        file_uri(Path::new("/run/media/a b/x?y#z%&.mbtiles")),
        "file:/run/media/a%20b/x%3Fy%23z%25%26.mbtiles?mode=ro&immutable=1"
    );
    assert_eq!(
        file_uri(Path::new("/é")),
        "file:/%C3%A9?mode=ro&immutable=1"
    );
}

#[test]
fn the_profile_schema_includes_the_shadow_tables() {
    let rows = profile_schema().expect("should succeed");
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    for n in [
        "rs_coverage",
        "rs_place",
        "rs_place_fts",
        "rs_place_fts_data",
        "rs_place_fts_idx",
        "rs_place_fts_docsize",
        "rs_place_fts_config",
        "rs_poi_rtree",
        "rs_poi_rtree_node",
        "rs_poi_rtree_rowid",
        "rs_poi_rtree_parent",
    ] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    // Every table the authorizer may open for a family is in the schema.
    for f in Family::ALL {
        for t in f.table_names() {
            assert!(names.contains(t), "{t}");
        }
    }
    // Nothing else (no autoindex needed: the keys are INTEGER PRIMARY KEY).
    assert_eq!(rows.len(), 11, "{names:?}");
}

#[test]
fn search_finds_places_through_the_hardened_connection() {
    use resilience_core::maps::search::{Kind, fts_query, tokens};
    let dir = TempDir::new("search");
    let path = dir.file("map.mbtiles");
    write_pack(&path);
    let pack = open(&path).expect("open");
    unchanged(dir.path(), || {
        let q = |s: &str| fts_query(&tokens(s)).expect("query");
        let names = |s: &str| -> Vec<String> {
            let mut v: Vec<String> = pack
                .search(&q(s))
                .expect("search")
                .into_iter()
                .map(|h| h.name)
                .collect();
            v.sort();
            v
        };
        // A prefix of the last word; accents and case do not matter.
        assert_eq!(names("tes"), ["Pharmacie du Test", "Testville"]);
        assert_eq!(names("ESSAI"), ["Rue de l'Essai"]);
        assert_eq!(names("pharmacie test"), ["Pharmacie du Test"]);
        assert!(names("nowhere").is_empty());
        let t = pack.search(&q("testville")).expect("search");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].kind, Kind::Settlement);
        assert_eq!(t[0].class, "town");
        assert!((t[0].at.lat() - 49.6116).abs() < 1e-6 && (t[0].at.lon() - 6.1319).abs() < 1e-6);
        let p = pack.search(&q("pharmacie")).expect("search");
        assert_eq!(p[0].near.as_deref(), Some("Testville"));
        // Hostile words are only words: no FTS5 syntax error, no rows.
        for hostile in ["\" OR \"", "NEAR(a b)", "a*:b", "^x -y"] {
            if let Some(f) = fts_query(&tokens(hostile)) {
                pack.search(&f).expect("only quoted words");
            }
        }
        assert!(pack.search("").expect("empty").is_empty());
    });
    // A pack without the search tables answers with nothing.
    let (_dir, r) = open_altered("nosearch", |c| {
        c.execute_batch("DROP TABLE rs_place_fts; DROP TABLE rs_place;")
            .expect("drop");
    });
    let pack = r.expect("still opens");
    assert!(pack.search("\"testville\"*").expect("no search").is_empty());
}

#[test]
fn nearby_reads_the_rtree_through_the_hardened_connection() {
    use resilience_core::maps::geo::LatLon;
    let dir = TempDir::new("nearby");
    let path = dir.file("map.mbtiles");
    write_pack(&path);
    let pack = open(&path).expect("open");
    unchanged(dir.path(), || {
        // The pharmacy is about 70 m from Testville's centre: in the first box.
        let at = LatLon::new(49.6116, 6.1319).expect("at");
        let found = pack.nearby(at).expect("nearby");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Pharmacie du Test");
        // 30 km away: the boxes widen to 10 km and stop; nothing.
        let far = LatLon::new(49.9, 6.1319).expect("far");
        assert!(pack.nearby(far).expect("nearby").is_empty());
    });
    // A pack whose R*Tree is gone answers with nothing.
    let (_dir, r) = open_altered("nortree", |c| {
        c.execute_batch("DROP TABLE rs_poi_rtree;").expect("drop");
    });
    let pack = r.expect("still opens");
    let at = LatLon::new(49.6116, 6.1319).expect("at");
    assert!(pack.nearby(at).expect("no nearby").is_empty());
}
