//! What the reader accepts in a map file's schema (ADR 0016 §7, "only
//! fixed statements on known names"). Pure: `pack.rs` reads the rows of
//! `sqlite_schema` and the column lists, this module decides.
//!
//! A hostile file can give `tiles`, `metadata` or an `rs_` name to a view,
//! a virtual table, or a table whose columns compute values. The rules:
//!
//! - **Triggers** anywhere: the file is refused. They could only fire on
//!   writes, which the connection cannot make (`query_only`, read-only,
//!   triggers disabled), but no MBTiles tool writes them, so one means the
//!   file was crafted.
//! - **The kind of every object is read from its SQL text**, the text
//!   SQLite itself parses when it loads the schema, never from the `type`
//!   column alone; a row whose `type` disagrees with its text is refused.
//! - **`metadata`** must be an ordinary table.
//! - **`tiles`** must be an ordinary table, or a view in one of the two
//!   de-duplicated layouts real tools write: mbutil and tilemaker (`map` +
//!   `images`), Planetiler's compact mode (`tiles_shallow` + `tiles_data`).
//!   In that case the view is **never used**: the reader queries the
//!   underlying ordinary tables with its own fixed join. A view's text
//!   decides nothing, so it cannot run anything. (Views are also disabled
//!   on the connection, see `pack.rs`.)
//! - The tables read must have the expected columns, none of them
//!   generated (`GENERATED ALWAYS AS`, evaluated on every read), and no
//!   default other than a literal (defaults are evaluated for rows written
//!   before a column was added). Their indexes must be over plain columns
//!   and not partial.
//! - Other objects (unknown tables, views, virtual tables, indexes) are
//!   tolerated and never touched: no statement of ours names them, views
//!   are disabled and the authorizer refuses to read them.
//! - The `rs_` tables are used only if their rows in `sqlite_schema` are
//!   exactly those that [`crate::profile::TABLES`] create (see
//!   [`Family`]); otherwise that feature is unavailable and the map still
//!   opens.

/// Most objects in a schema (SQLite's `SQLITE_LIMIT_SCHEMA` is set to the
/// same value in `pack.rs`, so a larger schema does not even load).
pub const MAX_OBJECTS: usize = 256;

/// One row of `sqlite_schema`, as stored.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SchemaRow {
    pub kind: String,
    pub name: String,
    pub table: String,
    pub sql: Option<String>,
}

/// The kind of object a `CREATE` statement makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Table,
    VirtualTable,
    Index,
    View,
    Trigger,
}

/// Why a schema was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    TooManyObjects,
    Trigger,
    /// A row whose text is not a statement SQLite stores, or whose `type`
    /// disagrees with it. Holds the cleaned object name.
    Crafted(String),
    Missing(&'static str),
    /// `metadata`, `tiles` or a table under it is not an ordinary table.
    NotATable(&'static str),
    /// A required column is missing from a table we read.
    MissingColumn(&'static str, &'static str),
    /// A generated column or a computed default in a table we read.
    ComputedColumn(&'static str),
    /// An expression or partial index on a table we read.
    ComputedIndex(&'static str),
    /// One of our fixed statements does not compile on this schema
    /// (an unknown collation, a type SQLite cannot use): cleaned message.
    Unusable(String),
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::TooManyObjects => {
                write!(f, "the file defines more than {MAX_OBJECTS} objects")
            }
            SchemaError::Trigger => write!(f, "the file contains triggers"),
            SchemaError::Crafted(n) => write!(f, "the file's definition of \"{n}\" is not genuine"),
            SchemaError::Missing(t) => write!(f, "the file has no \"{t}\" table"),
            SchemaError::NotATable(t) => write!(f, "\"{t}\" is not an ordinary table"),
            SchemaError::MissingColumn(t, c) => write!(f, "\"{t}\" has no column \"{c}\""),
            SchemaError::ComputedColumn(t) => write!(f, "\"{t}\" has computed columns"),
            SchemaError::ComputedIndex(t) => write!(f, "\"{t}\" has computed indexes"),
            SchemaError::Unusable(m) => write!(f, "the tables cannot be read ({m})"),
        }
    }
}

impl std::error::Error for SchemaError {}

/// The first keywords of a statement, as SQLite's tokenizer sees them:
/// white space and both kinds of comment are skipped.
fn keywords(sql: &str, n: usize) -> Vec<String> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() && out.len() < n {
        match b[i] {
            b' ' | b'\t' | b'\n' | b'\x0c' | b'\r' => i += 1,
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                out.push(sql[start..i].to_ascii_uppercase());
            }
            // Anything else ends the keyword prefix (a name, a parenthesis).
            _ => break,
        }
    }
    out
}

/// The kind of object `sql` creates, or `None` if it is not one of the
/// statements SQLite keeps in its schema.
pub fn statement_kind(sql: &str) -> Option<Kind> {
    // SQLite only parses schema text that starts with "CR" (any case).
    if !sql.as_bytes().get(..2)?.eq_ignore_ascii_case(b"CR") {
        return None;
    }
    let k = keywords(sql, 4);
    let mut w = k.iter().map(String::as_str);
    if w.next() != Some("CREATE") {
        return None;
    }
    let mut next = w.next();
    if matches!(next, Some("TEMP" | "TEMPORARY")) {
        next = w.next();
    }
    match next? {
        "TABLE" => Some(Kind::Table),
        "VIRTUAL" => (w.next() == Some("TABLE")).then_some(Kind::VirtualTable),
        "UNIQUE" => (w.next() == Some("INDEX")).then_some(Kind::Index),
        "INDEX" => Some(Kind::Index),
        "VIEW" => Some(Kind::View),
        "TRIGGER" => Some(Kind::Trigger),
        _ => None,
    }
}

/// How tiles are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileLayout {
    /// `tiles` is a table (tippecanoe, our builder).
    Table,
    /// `tiles` is a view over `map` and `images` (mbutil, tilemaker).
    MapImages,
    /// `tiles` is a view over `tiles_shallow` and `tiles_data` (Planetiler).
    ShallowData,
}

impl TileLayout {
    /// Fixed query for one tile: parameters zoom, column, row (TMS).
    pub fn tile_sql(self) -> &'static str {
        match self {
            TileLayout::Table => {
                "SELECT tile_data FROM tiles \
                 WHERE zoom_level = ?1 AND tile_column = ?2 AND tile_row = ?3 LIMIT 1"
            }
            TileLayout::MapImages => {
                "SELECT images.tile_data FROM map JOIN images ON images.tile_id = map.tile_id \
                 WHERE map.zoom_level = ?1 AND map.tile_column = ?2 AND map.tile_row = ?3 LIMIT 1"
            }
            TileLayout::ShallowData => {
                "SELECT tiles_data.tile_data FROM tiles_shallow \
                 JOIN tiles_data ON tiles_data.tile_data_id = tiles_shallow.tile_data_id \
                 WHERE tiles_shallow.zoom_level = ?1 AND tiles_shallow.tile_column = ?2 \
                 AND tiles_shallow.tile_row = ?3 LIMIT 1"
            }
        }
    }

    /// Fixed query counting the tiles.
    pub fn count_sql(self) -> &'static str {
        match self {
            TileLayout::Table => "SELECT count(*) FROM tiles",
            TileLayout::MapImages => "SELECT count(*) FROM map",
            TileLayout::ShallowData => "SELECT count(*) FROM tiles_shallow",
        }
    }

    /// The ordinary tables read and the columns each must have.
    pub fn tables(self) -> &'static [(&'static str, &'static [&'static str])] {
        const TILE: [&str; 3] = ["zoom_level", "tile_column", "tile_row"];
        match self {
            TileLayout::Table => &[("tiles", &[TILE[0], TILE[1], TILE[2], "tile_data"])],
            TileLayout::MapImages => &[
                ("map", &[TILE[0], TILE[1], TILE[2], "tile_id"]),
                ("images", &["tile_data", "tile_id"]),
            ],
            TileLayout::ShallowData => &[
                (
                    "tiles_shallow",
                    &[TILE[0], TILE[1], TILE[2], "tile_data_id"],
                ),
                ("tiles_data", &["tile_data_id", "tile_data"]),
            ],
        }
    }
}

/// The `metadata` table and its columns.
pub const METADATA: (&str, &[&str]) = ("metadata", &["name", "value"]);

/// Check every row and decide the tile layout.
pub fn classify(rows: &[SchemaRow]) -> Result<TileLayout, SchemaError> {
    if rows.len() > MAX_OBJECTS {
        return Err(SchemaError::TooManyObjects);
    }
    for r in rows {
        let kind = match &r.sql {
            // Indexes SQLite makes for UNIQUE and PRIMARY KEY have no text.
            None if r.kind == "index" && r.name.starts_with("sqlite_autoindex_") => continue,
            None => return Err(SchemaError::Crafted(crate::text::clean_short(&r.name, 64))),
            Some(sql) => statement_kind(sql),
        };
        let agrees = matches!(
            (r.kind.as_str(), kind),
            ("table", Some(Kind::Table | Kind::VirtualTable))
                | ("index", Some(Kind::Index))
                | ("view", Some(Kind::View))
                | ("trigger", Some(Kind::Trigger))
        );
        if kind == Some(Kind::Trigger) || r.kind == "trigger" {
            return Err(SchemaError::Trigger);
        }
        if !agrees {
            return Err(SchemaError::Crafted(crate::text::clean_short(&r.name, 64)));
        }
    }
    let kind_of = |name: &str| {
        rows.iter()
            .find(|r| r.name.eq_ignore_ascii_case(name) && r.kind != "index")
            .map(|r| r.sql.as_deref().and_then(statement_kind))
    };
    let is_table = |name: &str| kind_of(name) == Some(Some(Kind::Table));
    match kind_of("metadata") {
        None => return Err(SchemaError::Missing("metadata")),
        Some(Some(Kind::Table)) => {}
        Some(_) => return Err(SchemaError::NotATable("metadata")),
    }
    match kind_of("tiles") {
        None => Err(SchemaError::Missing("tiles")),
        Some(Some(Kind::Table)) => Ok(TileLayout::Table),
        Some(Some(Kind::View)) if is_table("map") && is_table("images") => {
            Ok(TileLayout::MapImages)
        }
        Some(Some(Kind::View)) if is_table("tiles_shallow") && is_table("tiles_data") => {
            Ok(TileLayout::ShallowData)
        }
        Some(_) => Err(SchemaError::NotATable("tiles")),
    }
}

/// One column as `pragma_table_xinfo` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// 0 ordinary, 1 hidden (virtual tables), 2 and 3 generated.
    pub hidden: i64,
    pub default: Option<String>,
}

/// Whether a default value is a plain literal: NULL, a number, or a string
/// without quotes inside.
fn literal_default(d: &str) -> bool {
    let d = d.trim();
    let number = d.strip_prefix(['-', '+']).unwrap_or(d);
    d.eq_ignore_ascii_case("NULL")
        || (!number.is_empty()
            && number.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            && number.bytes().filter(|&b| b == b'.').count() <= 1)
        || (d.len() >= 2
            && d.starts_with('\'')
            && d.ends_with('\'')
            && !d[1..d.len() - 1].contains('\''))
}

/// Check the columns of a table we read.
pub fn check_columns(
    table: &'static str,
    required: &[&'static str],
    columns: &[Column],
) -> Result<(), SchemaError> {
    for c in columns {
        if c.hidden != 0 || c.default.as_deref().is_some_and(|d| !literal_default(d)) {
            return Err(SchemaError::ComputedColumn(table));
        }
    }
    for r in required {
        if !columns.iter().any(|c| c.name.eq_ignore_ascii_case(r)) {
            return Err(SchemaError::MissingColumn(table, r));
        }
    }
    Ok(())
}

/// A feature backed by `rs_` tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Coverage,
    Place,
    PlaceFts,
    PoiRtree,
}

impl Family {
    pub const ALL: [Family; 4] = [
        Family::Coverage,
        Family::Place,
        Family::PlaceFts,
        Family::PoiRtree,
    ];

    /// The family of the object a row belongs to, by the table it is on.
    /// FTS5 and R*Tree shadow tables are named after their virtual table.
    pub fn of(row: &SchemaRow) -> Option<Family> {
        let t = row.table.as_str();
        let under = |base: &str| {
            t.len() >= base.len()
                && t[..base.len()].eq_ignore_ascii_case(base)
                && (t.len() == base.len() || t.as_bytes()[base.len()] == b'_')
        };
        if t.eq_ignore_ascii_case("rs_coverage") {
            Some(Family::Coverage)
        } else if t.eq_ignore_ascii_case("rs_place") {
            Some(Family::Place)
        } else if under("rs_place_fts") {
            Some(Family::PlaceFts)
        } else if under("rs_poi_rtree") {
            Some(Family::PoiRtree)
        } else {
            None
        }
    }

    /// Tables of this family that the reader (and SQLite's modules, for
    /// the shadow tables) may read, for the authorizer.
    pub fn table_names(self) -> &'static [&'static str] {
        match self {
            Family::Coverage => &["rs_coverage"],
            Family::Place => &["rs_place"],
            Family::PlaceFts => &[
                "rs_place_fts",
                "rs_place_fts_data",
                "rs_place_fts_idx",
                "rs_place_fts_docsize",
                "rs_place_fts_config",
            ],
            Family::PoiRtree => &[
                "rs_poi_rtree",
                "rs_poi_rtree_node",
                "rs_poi_rtree_rowid",
                "rs_poi_rtree_parent",
            ],
        }
    }
}

/// How one family compares with the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FamilyState {
    Exact,
    Absent,
    /// The first object (cleaned name) that differs, is missing or extra.
    Differs(String),
}

/// Compare the file's rows of `family` with the profile's.
pub fn compare_family(family: Family, file: &[SchemaRow], profile: &[SchemaRow]) -> FamilyState {
    let pick = |rows: &[SchemaRow]| {
        let mut v: Vec<SchemaRow> = rows
            .iter()
            .filter(|r| Family::of(r) == Some(family))
            .cloned()
            .collect();
        v.sort();
        v
    };
    let (have, want) = (pick(file), pick(profile));
    if have.is_empty() {
        return FamilyState::Absent;
    }
    for w in &want {
        if !have.contains(w) {
            return FamilyState::Differs(crate::text::clean_short(&w.name, 64));
        }
    }
    match have.iter().find(|h| !want.contains(h)) {
        Some(extra) => FamilyState::Differs(crate::text::clean_short(&extra.name, 64)),
        None => FamilyState::Exact,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, name: &str, table: &str, sql: Option<&str>) -> SchemaRow {
        SchemaRow {
            kind: kind.into(),
            name: name.into(),
            table: table.into(),
            sql: sql.map(Into::into),
        }
    }

    fn table(name: &str) -> SchemaRow {
        row(
            "table",
            name,
            name,
            Some(&format!("CREATE TABLE {name} (a, b)")),
        )
    }

    #[test]
    fn statement_kinds_follow_sqlites_tokenizer() {
        use Kind::*;
        for (sql, kind) in [
            ("CREATE TABLE t(a)", Some(Table)),
            ("create table t(a)", Some(Table)),
            ("CREATE TEMP TABLE t(a)", Some(Table)),
            (
                "CREATE/*x*/VIRTUAL\n\tTABLE t USING fts5(a)",
                Some(VirtualTable),
            ),
            ("CREATE -- c\nVIEW v AS SELECT 1", Some(View)),
            ("CREATE UNIQUE INDEX i ON t(a)", Some(Index)),
            ("CREATE INDEX i ON t(a)", Some(Index)),
            (
                "CREATE TRIGGER x AFTER INSERT ON t BEGIN SELECT 1; END",
                Some(Trigger),
            ),
            ("CREATE TEMPORARY TRIGGER x", Some(Trigger)),
            (" CREATE TABLE t(a)", None),
            ("/*x*/CREATE TABLE t(a)", None),
            ("CREATE VIRTUAL t", None),
            ("CREATE UNIQUE TABLE t", None),
            ("SELECT 1", None),
            ("CR", None),
            ("", None),
        ] {
            assert_eq!(statement_kind(sql), kind, "{sql:?}");
        }
    }

    #[test]
    fn layouts_are_recognised() {
        let base = [table("metadata")];
        let mut t = base.to_vec();
        t.push(table("tiles"));
        assert_eq!(classify(&t), Ok(TileLayout::Table));
        let view = row(
            "view",
            "tiles",
            "tiles",
            Some("CREATE VIEW tiles AS SELECT 1"),
        );
        let mut m = base.to_vec();
        m.extend([view.clone(), table("map"), table("images")]);
        assert_eq!(classify(&m), Ok(TileLayout::MapImages));
        let mut s = base.to_vec();
        s.extend([view.clone(), table("tiles_shallow"), table("tiles_data")]);
        assert_eq!(classify(&s), Ok(TileLayout::ShallowData));
        let mut lone = base.to_vec();
        lone.push(view);
        assert_eq!(classify(&lone), Err(SchemaError::NotATable("tiles")));
        assert_eq!(classify(&base), Err(SchemaError::Missing("tiles")));
        assert_eq!(
            classify(&[table("tiles")]),
            Err(SchemaError::Missing("metadata"))
        );
    }

    #[test]
    fn wrong_kinds_and_crafted_rows_are_refused() {
        let vt = row(
            "table",
            "tiles",
            "tiles",
            Some("CREATE VIRTUAL TABLE tiles USING fts5(x)"),
        );
        assert_eq!(
            classify(&[table("metadata"), vt]),
            Err(SchemaError::NotATable("tiles"))
        );
        let mv = row(
            "view",
            "metadata",
            "metadata",
            Some("CREATE VIEW metadata AS SELECT 1"),
        );
        assert_eq!(
            classify(&[mv, table("tiles")]),
            Err(SchemaError::NotATable("metadata"))
        );
        let liar = row("index", "x", "tiles", Some("CREATE VIEW x AS SELECT 1"));
        assert_eq!(
            classify(&[table("metadata"), table("tiles"), liar]),
            Err(SchemaError::Crafted("x".into()))
        );
        let hidden = row(
            "table",
            "y",
            "y",
            Some("CREATE TRIGGER y AFTER INSERT ON t BEGIN END"),
        );
        assert_eq!(
            classify(&[table("metadata"), table("tiles"), hidden]),
            Err(SchemaError::Trigger)
        );
        let no_sql = row("table", "z", "z", None);
        assert!(matches!(
            classify(&[table("metadata"), table("tiles"), no_sql]),
            Err(SchemaError::Crafted(_))
        ));
        let auto = row("index", "sqlite_autoindex_metadata_1", "metadata", None);
        assert!(classify(&[table("metadata"), table("tiles"), auto]).is_ok());
        let many: Vec<SchemaRow> = (0..=MAX_OBJECTS).map(|i| table(&format!("t{i}"))).collect();
        assert_eq!(classify(&many), Err(SchemaError::TooManyObjects));
    }

    #[test]
    fn computed_columns_are_refused() {
        let col = |name: &str, hidden: i64, default: Option<&str>| Column {
            name: name.into(),
            hidden,
            default: default.map(Into::into),
        };
        let ok = [col("name", 0, None), col("value", 0, Some("''"))];
        assert_eq!(check_columns("metadata", METADATA.1, &ok), Ok(()));
        for d in ["NULL", "0", "-1.5", "'x'", "+3"] {
            assert_eq!(
                check_columns(
                    "metadata",
                    METADATA.1,
                    &[col("name", 0, None), col("value", 0, Some(d))]
                ),
                Ok(()),
                "{d}"
            );
        }
        for d in [
            "(randomblob(1000000))",
            "zeroblob(5)",
            "'a' || 'b'",
            "1e9",
            "x'00'",
            "'it''s'",
        ] {
            assert_eq!(
                check_columns(
                    "metadata",
                    METADATA.1,
                    &[col("name", 0, None), col("value", 0, Some(d))]
                ),
                Err(SchemaError::ComputedColumn("metadata")),
                "{d}"
            );
        }
        for hidden in [2, 3] {
            assert_eq!(
                check_columns(
                    "metadata",
                    METADATA.1,
                    &[col("name", 0, None), col("value", hidden, None)]
                ),
                Err(SchemaError::ComputedColumn("metadata"))
            );
        }
        assert_eq!(
            check_columns("metadata", METADATA.1, &[col("name", 0, None)]),
            Err(SchemaError::MissingColumn("metadata", "value"))
        );
    }

    #[test]
    fn families_compare_exactly() {
        let p = |name: &str, sql: &str| row("table", name, name, Some(sql));
        let profile = vec![
            p("rs_coverage", "CREATE TABLE rs_coverage (ring)"),
            p(
                "rs_place_fts",
                "CREATE VIRTUAL TABLE rs_place_fts USING fts5(name)",
            ),
            p("rs_place_fts_data", "CREATE TABLE 'rs_place_fts_data'(id)"),
        ];
        assert_eq!(
            compare_family(Family::Coverage, &profile, &profile),
            FamilyState::Exact
        );
        assert_eq!(
            compare_family(Family::PlaceFts, &profile, &profile),
            FamilyState::Exact
        );
        assert_eq!(
            compare_family(Family::Place, &profile, &profile),
            FamilyState::Absent
        );
        let mut one_char = profile.clone();
        one_char[0].sql = Some("CREATE TABLE rs_coverage (rings)".into());
        assert_eq!(
            compare_family(Family::Coverage, &one_char, &profile),
            FamilyState::Differs("rs_coverage".into())
        );
        let mut missing_shadow = profile.clone();
        missing_shadow.pop();
        assert_eq!(
            compare_family(Family::PlaceFts, &missing_shadow, &profile),
            FamilyState::Differs("rs_place_fts_data".into())
        );
        let mut extra = profile.clone();
        extra.push(row(
            "index",
            "i",
            "rs_coverage",
            Some("CREATE INDEX i ON rs_coverage(ring)"),
        ));
        assert_eq!(
            compare_family(Family::Coverage, &extra, &profile),
            FamilyState::Differs("i".into())
        );
        assert_eq!(Family::of(&table("rs_place_ftsx")), None);
        assert_eq!(
            Family::of(&table("rs_place_fts_idx")),
            Some(Family::PlaceFts)
        );
        assert_eq!(Family::of(&table("RS_PLACE")), Some(Family::Place));
    }
}
