use std::io::Write;

use rusqlite::{Connection, OpenFlags};

use super::*;
use crate::fixtures::{TIPPECANOE_SCHEMA, TempDir, sample_tile};
use crate::inflate::gzip;
use crate::mapmaker::rows::{Kind, Row};

fn row(kind: Kind, class: &str, name: &str, lon: i32, lat: i32, rank: i64) -> Row {
    Row {
        kind,
        merge: false,
        class: class.into(),
        name: name.into(),
        lon,
        lat,
        rank,
        bbox: None,
    }
}

fn seg(kind: Kind, class: &str, name: &str, at: (i32, i32), rank: i64, bbox: [i32; 4]) -> Row {
    Row {
        merge: true,
        bbox: Some(bbox),
        ..row(kind, class, name, at.0, at.1, rank)
    }
}

/// The side file: settlements, street segments (two merged, one near
/// another village, three without any settlement nearby), POIs, natural.
fn sample_rows() -> Vec<Row> {
    use Kind::*;
    vec![
        seg(
            Street,
            "residential",
            "Rue de l'Essai",
            (6_001_000, 49_001_000),
            6,
            [6_000_000, 49_000_500, 6_002_000, 49_001_500],
        ),
        row(Settlement, "town", "Testville", 6_000_000, 49_000_000, 163),
        row(
            Poi,
            "pharmacy",
            "Pharmacie du Test",
            6_000_500,
            49_000_500,
            0,
        ),
        seg(
            Street,
            "tertiary",
            "Rue de l'Essai",
            (6_003_000, 49_002_000),
            4,
            [6_002_000, 49_001_500, 6_004_000, 49_002_500],
        ),
        row(
            Settlement,
            "village",
            "Petit-Test",
            6_050_000,
            49_000_000,
            299,
        ),
        seg(
            Street,
            "residential",
            "Rue de l'Essai",
            (6_049_000, 49_000_000),
            6,
            [6_048_000, 48_999_500, 6_050_000, 49_000_500],
        ),
        row(
            Settlement,
            "hamlet",
            "Loinville",
            7_000_000,
            49_000_000,
            399,
        ),
        seg(
            Street,
            "residential",
            "Route perdue",
            (8_010_000, 49_000_000),
            6,
            [8_009_000, 48_999_000, 8_011_000, 49_001_000],
        ),
        seg(
            Street,
            "residential",
            "Route perdue",
            (8_000_000, 49_000_000),
            6,
            [7_999_000, 48_999_000, 8_001_000, 49_001_000],
        ),
        seg(
            Street,
            "residential",
            "Route perdue",
            (8_200_000, 49_000_000),
            6,
            [8_199_000, 48_999_000, 8_201_000, 49_001_000],
        ),
        row(Poi, "drinking_water", "", 6_000_800, 49_000_200, 1),
        row(Settlement, "suburb", "Quartier", 6_001_000, 49_000_000, 499),
        row(Natural, "peak", "Mont Test", 6_020_000, 49_030_000, 2),
        Row {
            bbox: Some([6_025_000, 49_015_000, 6_035_000, 49_025_000]),
            ..row(Natural, "lake", "Lac Test", 6_030_000, 49_020_000, 0)
        },
    ]
}

const POLY: &str = "testland\n1\n5.9 48.9\n6.5 48.9\n6.5 49.5\n5.9 49.5\nEND\nEND\n";

struct Inputs3 {
    dir: TempDir,
}

impl Inputs3 {
    fn new(tag: &str, rows: &[Row], bad_tile: bool) -> Inputs3 {
        let dir = TempDir::new(tag);
        let mut f = std::fs::File::create(dir.file("rows.jsonl")).expect("rows");
        for r in rows {
            serde_json::to_writer(&mut f, r).expect("row");
            f.write_all(b"\n").expect("newline");
        }
        std::fs::write(dir.file("cover.poly"), POLY).expect("poly");
        let c = Connection::open(dir.file("tiles.mbtiles")).expect("tiles");
        c.execute_batch(TIPPECANOE_SCHEMA).expect("schema");
        for (k, v) in [
            ("name", "tiles.mbtiles"),
            ("bounds", "5.8,48.8,8.3,49.6"),
            ("generator_options", "tippecanoe -o /build/tiles.mbtiles"),
            (
                "json",
                r#"{"vector_layers":[{"id":"places","fields":{"name":"String"}}],"tilestats":{"layerCount":1}}"#,
            ),
        ] {
            c.execute("INSERT INTO metadata VALUES (?1, ?2)", [k, v])
                .expect("meta");
        }
        // Inserted out of order: finish sorts.
        for (z, x, y) in [(1, 1, 1), (0, 0, 0), (1, 0, 1)] {
            let data = if bad_tile && z == 1 && x == 1 {
                gzip(b"\x1a\x05garbage")
            } else {
                gzip(&sample_tile(&format!("T{z}-{x}-{y}")))
            };
            c.execute(
                "INSERT INTO tiles VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![z, x, y, data],
            )
            .expect("tile");
        }
        c.close().expect("close");
        Inputs3 { dir }
    }

    fn run(&self, out: &str) -> Result<Stats, Error> {
        let (t, r, p) = (
            self.dir.file("tiles.mbtiles"),
            self.dir.file("rows.jsonl"),
            self.dir.file("cover.poly"),
        );
        super::run(
            &Inputs {
                tiles: &t,
                rows: &r,
                poly: &p,
            },
            &describe(),
            &self.dir.file(out),
        )
    }
}

fn describe() -> Describe {
    Describe {
        name: "Testland".into(),
        description: Some("Synthetic test region".into()),
        data_date: "2026-01-01".into(),
        source: "testland.osm.pbf sha256:00".into(),
        coverage: "Testland (synthetic), part of the box".into(),
        license: "ODbL-1.0".into(),
        attribution: "(c) OpenStreetMap contributors".into(),
        builder: "resilience-mapmaker test".into(),
    }
}

fn open(path: &Path) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("open pack")
}

type PlaceRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    i32,
    i32,
    i64,
    Option<Vec<u8>>,
);

fn places(c: &Connection) -> Vec<PlaceRow> {
    let mut st = c
        .prepare(
            "SELECT kind, class, name, fold, near, lon, lat, rank, bbox FROM rs_place ORDER BY id",
        )
        .expect("select");
    st.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
        ))
    })
    .expect("rows")
    .collect::<Result<_, _>>()
    .expect("rows")
}

fn bbox(w: i32, s: i32, e: i32, n: i32) -> Option<Vec<u8>> {
    Some(profile::encode_bbox(MicroBox {
        south_west: Micro::new(w, s).expect("sw"),
        north_east: Micro::new(e, n).expect("ne"),
    }))
}

#[test]
fn rows_are_merged_sorted_and_near_their_settlement() {
    let inputs = Inputs3::new("finish-rows", &sample_rows(), false);
    let stats = inputs.run("pack.mbtiles").expect("finish");
    assert_eq!(stats.rows_by_kind, [4, 4, 2, 2]);
    assert_eq!(stats.segments, 6);
    assert_eq!(stats.rtree, 3);
    let c = open(&inputs.dir.file("pack.mbtiles"));
    let s = |v: &str| v.to_string();
    let n = |v: &str| Some(v.to_string());
    let want: Vec<PlaceRow> = vec![
        (
            s("settlement"),
            s("town"),
            s("Testville"),
            s("testville"),
            n("Petit-Test"),
            6_000_000,
            49_000_000,
            163,
            None,
        ),
        (
            s("settlement"),
            s("village"),
            s("Petit-Test"),
            s("petit test"),
            n("Testville"),
            6_050_000,
            49_000_000,
            299,
            None,
        ),
        (
            s("settlement"),
            s("hamlet"),
            s("Loinville"),
            s("loinville"),
            None,
            7_000_000,
            49_000_000,
            399,
            None,
        ),
        (
            s("settlement"),
            s("suburb"),
            s("Quartier"),
            s("quartier"),
            n("Testville"),
            6_001_000,
            49_000_000,
            499,
            None,
        ),
        // Two segments merged: best class, union of boxes, the point
        // nearest the box's centre (a tie, broken westwards).
        (
            s("street"),
            s("tertiary"),
            s("Rue de l'Essai"),
            s("rue de l essai"),
            n("Testville"),
            6_001_000,
            49_001_000,
            4,
            bbox(6_000_000, 49_000_500, 6_004_000, 49_002_500),
        ),
        // No settlement within 10 km: merged by 0.1 degree cell.
        (
            s("street"),
            s("residential"),
            s("Route perdue"),
            s("route perdue"),
            None,
            8_000_000,
            49_000_000,
            6,
            bbox(7_999_000, 48_999_000, 8_011_000, 49_001_000),
        ),
        (
            s("street"),
            s("residential"),
            s("Route perdue"),
            s("route perdue"),
            None,
            8_200_000,
            49_000_000,
            6,
            bbox(8_199_000, 48_999_000, 8_201_000, 49_001_000),
        ),
        (
            s("street"),
            s("residential"),
            s("Rue de l'Essai"),
            s("rue de l essai"),
            n("Petit-Test"),
            6_049_000,
            49_000_000,
            6,
            bbox(6_048_000, 48_999_500, 6_050_000, 49_000_500),
        ),
        (
            s("poi"),
            s("pharmacy"),
            s("Pharmacie du Test"),
            s("pharmacie du test"),
            n("Testville"),
            6_000_500,
            49_000_500,
            0,
            None,
        ),
        (
            s("poi"),
            s("drinking_water"),
            s(""),
            s(""),
            n("Testville"),
            6_000_800,
            49_000_200,
            1,
            None,
        ),
        (
            s("natural"),
            s("lake"),
            s("Lac Test"),
            s("lac test"),
            n("Petit-Test"),
            6_030_000,
            49_020_000,
            0,
            bbox(6_025_000, 49_015_000, 6_035_000, 49_025_000),
        ),
        (
            s("natural"),
            s("peak"),
            s("Mont Test"),
            s("mont test"),
            n("Testville"),
            6_020_000,
            49_030_000,
            2,
            None,
        ),
    ];
    assert_eq!(places(&c), want);
    // Full-text search with prefixes, and the R*Tree over POIs and peaks.
    let fts = |q: &str| -> Vec<i64> {
        let mut st = c
            .prepare("SELECT rowid FROM rs_place_fts WHERE rs_place_fts MATCH ?1 ORDER BY rowid")
            .expect("fts");
        st.query_map([q], |r| r.get(0))
            .expect("q")
            .collect::<Result<_, _>>()
            .expect("ids")
    };
    assert_eq!(fts("\"essai\""), vec![5, 8]);
    assert_eq!(fts("\"te\"*"), vec![1, 2, 9, 11, 12]);
    assert_eq!(fts("\"etienne\""), Vec::<i64>::new());
    let near: Vec<i64> = c
        .prepare(
            "SELECT id FROM rs_poi_rtree WHERE min_lon <= 6001000 AND max_lon >= 5999000
             AND min_lat <= 49001000 AND max_lat >= 48999000 ORDER BY id",
        )
        .expect("rtree")
        .query_map([], |r| r.get(0))
        .expect("q")
        .collect::<Result<_, _>>()
        .expect("ids");
    assert_eq!(near, vec![9, 10]);
}

#[test]
fn metadata_and_coverage_are_fixed() {
    let inputs = Inputs3::new("finish-meta", &sample_rows(), false);
    let stats = inputs.run("pack.mbtiles").expect("finish");
    let path = inputs.dir.file("pack.mbtiles");
    let c = open(&path);
    let meta: Vec<(String, String)> = c
        .prepare("SELECT name, value FROM metadata ORDER BY rowid")
        .expect("meta")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("q")
        .collect::<Result<_, _>>()
        .expect("rows");
    let keys: Vec<&str> = meta.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "attribution",
            "bounds",
            "center",
            "description",
            "format",
            "json",
            "maxzoom",
            "minzoom",
            "name",
            "resilience_builder",
            "resilience_coverage",
            "resilience_data_date",
            "resilience_license",
            "resilience_profile",
            "resilience_source",
            "type"
        ]
    );
    let get = |k: &str| meta.iter().find(|(m, _)| m == k).map(|(_, v)| v.as_str());
    assert_eq!(get("bounds"), Some("5.800000,48.800000,8.300000,49.600000"));
    // The most important settlement inside the coverage.
    assert_eq!(get("center"), Some("6.000000,49.000000,10"));
    assert_eq!(
        get("json"),
        Some(r#"{"vector_layers":[{"fields":{"name":"String"},"id":"places"}]}"#)
    );
    assert_eq!((get("minzoom"), get("maxzoom")), (Some("0"), Some("1")));
    // The pack opens with the reader, every feature available.
    let pack = Pack::open(&path).expect("reader");
    assert!(pack.capabilities().search.is_ok());
    assert!(pack.capabilities().nearby.is_ok());
    let cov = pack.coverage().expect("coverage").expect("some");
    assert_eq!(cov.rings.len(), 1);
    assert_eq!(cov.rings[0].points.len(), 4);
    assert_eq!(pack.metadata().name, "Testland");
    // Tiles are in (z, x, y) order.
    let order: Vec<(i64, i64, i64)> = c
        .prepare("SELECT zoom_level, tile_column, tile_row FROM tiles ORDER BY rowid")
        .expect("tiles")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("q")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(order, [(0, 0, 0), (1, 0, 1), (1, 1, 1)]);
    assert_eq!(stats.tiles.by_zoom.iter().map(|z| z.1).sum::<u64>(), 3);
    assert!(stats.tiles.most_features.0 == 2);
    assert!(stats.report().contains("finish: z1: 2 tiles"));
    let page: i64 = c
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .expect("page");
    assert_eq!(page, i64::from(PAGE_SIZE));
    let app: i64 = c
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .expect("app");
    assert_eq!(app, i64::from(APPLICATION_ID));
}

#[test]
fn two_runs_give_identical_bytes() {
    // The same rows in another order give the same pack too.
    let rows = sample_rows();
    let mut reversed = rows.clone();
    reversed.reverse();
    let a = Inputs3::new("finish-det-a", &rows, false);
    let b = Inputs3::new("finish-det-b", &reversed, false);
    a.run("one.mbtiles").expect("finish");
    a.run("two.mbtiles").expect("finish");
    b.run("three.mbtiles").expect("finish");
    let read = |i: &Inputs3, n: &str| std::fs::read(i.dir.file(n)).expect("read");
    let one = read(&a, "one.mbtiles");
    assert_eq!(one, read(&a, "two.mbtiles"));
    assert_eq!(one, read(&b, "three.mbtiles"));
    assert_eq!(one.len() % PAGE_SIZE as usize, 0);
}

#[test]
fn bad_inputs_fail_the_build() {
    let bad = Inputs3::new("finish-bad", &sample_rows(), true);
    let e = bad.run("pack.mbtiles").expect_err("bad tile");
    assert!(e.0.contains("tile 1/1/1"), "{e}");
    let ok = Inputs3::new("finish-exists", &sample_rows(), false);
    std::fs::write(ok.dir.file("pack.mbtiles"), b"").expect("touch");
    assert!(
        ok.run("pack.mbtiles")
            .expect_err("exists")
            .0
            .contains("exists")
    );
    std::fs::write(ok.dir.file("rows.jsonl"), b"{\"kind\":\"poi\"}\n").expect("rows");
    assert!(
        ok.run("other.mbtiles")
            .expect_err("bad row")
            .0
            .contains("rows line 1")
    );
}

#[test]
fn nearest_settlement_rules() {
    let m = |lon: f64, lat: f64| Micro::from_degrees(lon, lat).expect("point");
    let idx = NearIndex::new(vec![
        (m(6.0, 49.0), 100, "A".into()),
        (m(6.1, 49.0), 200, "B".into()),
        (m(179.99, 0.0), 100, "East".into()),
        (m(6.05, 49.0), 300, "Tie".into()),
        (m(6.05, 49.0), 100, "Tie2".into()),
    ]);
    assert_eq!(idx.len(), 5);
    assert_eq!(idx.nearest(m(6.01, 49.0), None).map(|x| x.1), Some("A"));
    assert_eq!(
        idx.nearest(m(6.0, 49.0), Some("A")).map(|x| x.1),
        Some("Tie2")
    );
    // Same place, two settlements: the lower rank.
    assert_eq!(idx.nearest(m(6.05, 49.0), None).map(|x| x.1), Some("Tie2"));
    // More than 10 km: none.
    assert_eq!(idx.nearest(m(6.3, 49.0), None), None);
    assert_eq!(idx.nearest(m(6.0, 49.1), None), None);
    // Across the antimeridian.
    assert_eq!(
        idx.nearest(m(-179.99, 0.0), None).map(|x| x.1),
        Some("East")
    );
    assert!(NearIndex::new(Vec::new()).is_empty());
}
