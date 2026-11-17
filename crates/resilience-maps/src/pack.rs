//! The hardened reader for one map pack file (ADR 0016 §7, "SQLite
//! hardening for every map file"). Map files are untrusted: files on a
//! stick are opened without any hash check, as the logged-in user.
//!
//! What [`Pack::open`] does, in order (each step names the ADR item):
//!
//! 1. **No symlinks, regular files only.** The directory part of the path
//!    is resolved (symlinks there are followed: SQLite's `NOFOLLOW` refuses
//!    a symlink in *any* component, and system paths such as
//!    `/run/current-system` have them). The file itself is opened with
//!    `O_NOFOLLOW | O_NONBLOCK` (a FIFO cannot block us) and must be a
//!    regular file. SQLite is then opened with `SQLITE_OPEN_NOFOLLOW` too.
//! 2. **The database header is checked by us before SQLite sees the file**
//!    ([`check_header`]): magic, page size, payload fractions, and the
//!    file-format bytes 18-19, which are 2 for a database in **WAL mode**:
//!    such a file is refused. A length that is not a whole number of pages,
//!    or shorter than the header says, is refused as truncated.
//! 3. **Read-only, immutable open**: `SQLITE_OPEN_READONLY | NOFOLLOW` and
//!    the URI parameters `mode=ro&immutable=1`. With `immutable`, SQLite
//!    takes no locks and never looks for or creates a journal, WAL, shm or
//!    lock file: nothing is ever written beside the file, and a stale
//!    journal or WAL beside it is ignored. The file's identity (device,
//!    inode) is compared with the descriptor opened in step 1, which is
//!    kept open.
//! 4. **Connection settings** ([`harden`]), each read back and refused if it
//!    did not take: `SQLITE_DBCONFIG_DEFENSIVE`; `TRUSTED_SCHEMA` off;
//!    triggers and views disabled (`ENABLE_TRIGGER`, `ENABLE_VIEW` off: no
//!    statement can run a view's SQL, ours never names one); ATTACH may
//!    neither create nor write; `PRAGMA query_only`, `cell_size_check`,
//!    `mmap_size=0`, `cache_size` 8 MiB, `temp_store=MEMORY` (no temporary
//!    files anywhere).
//! 5. **`sqlite3_limit` caps** ([`LIMITS`]): those of the ADR, plus at most
//!    [`schema::MAX_OBJECTS`] schema objects and no worker threads.
//! 6. **A progress handler** aborts any statement past its [`Budget`] of
//!    time and virtual-machine steps: [`TILE_BUDGET`] for a tile,
//!    [`QUERY_BUDGET`] for everything else, so a crafted schema or a huge
//!    unindexed table cannot hang the viewer. It also bounds loading the
//!    schema and `PRAGMA quick_check`.
//! 7. **An authorizer** refuses, at prepare time, every action except
//!    SELECT, reading the tables this reader knows (and that this file
//!    passed the checks for), a short list of functions and pragmas, and
//!    nothing at all when the access comes through a view or trigger.
//!    `load_extension`, ATTACH, recursive CTEs and any write are refused.
//! 8. **Only fixed SQL on known names.** `sqlite_schema` is read first and
//!    judged by [`schema`]; the tables read are checked for computed
//!    columns and indexes; the fixed statements are compiled once to prove
//!    they work on this schema.
//! 9. **Extension loading is impossible**: rusqlite's `load_extension`
//!    feature is off (there is no API to enable it), and the bundled SQLite
//!    is compiled with `SQLITE_OMIT_LOAD_EXTENSION` (`.cargo/config.toml`),
//!    so the SQL function does not even exist.
//!
//! [`Pack::quick_check`] runs `PRAGMA quick_check`, bounded by
//! [`QUICK_CHECK_BUDGET`]. It is the caller's choice when to run it: ADR
//! §7 asks for it on an unchecked file up to [`QUICK_CHECK_AUTO_MAX`].

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use resilience_core::maps::geo::{TileId, TmsTile};
use resilience_core::maps::mvt;
use rusqlite::config::DbConfig;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, ErrorCode, OpenFlags};

use crate::inflate::{self, MAX_COMPRESSED, TileError};
use crate::metadata::{Collector, Metadata, MetadataError, Profile, RawValue};
use crate::profile::{self, Micro};
use crate::schema::{self, Family, FamilyState, SchemaError, SchemaRow, TileLayout};
use crate::text::clean_short;

/// Time and virtual-machine steps one operation may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub time: Duration,
    /// SQLite VDBE operations (for `quick_check`: b-tree pages checked).
    pub ops: u64,
}

/// One tile read (ADR: 200 ms). An indexed lookup needs a few dozen steps.
pub const TILE_BUDGET: Budget = Budget {
    time: Duration::from_millis(200),
    ops: 2_000_000,
};
/// Opening (schema, metadata), coverage, counting, later search (ADR: 2 s).
pub const QUERY_BUDGET: Budget = Budget {
    time: Duration::from_secs(2),
    ops: 50_000_000,
};
/// `PRAGMA quick_check`: reads the whole file, so more time.
pub const QUICK_CHECK_BUDGET: Budget = Budget {
    time: Duration::from_secs(60),
    ops: u64::MAX,
};
/// Files up to this size get `quick_check` before use when not checked
/// against the catalog (ADR §7); larger ones are offered the check.
pub const QUICK_CHECK_AUTO_MAX: u64 = 256 * 1024 * 1024;

/// The progress handler runs every this many VDBE operations.
const OPS_PER_CALL: i32 = 1000;

/// `sqlite3_limit` caps (ADR §7, plus SCHEMA and WORKER_THREADS).
pub const LIMITS: [(Limit, i32); 9] = [
    // Bounds a tile blob, any metadata value and any schema text.
    (Limit::SQLITE_LIMIT_LENGTH, MAX_COMPRESSED as i32),
    // Our statements and every CREATE text of the schema.
    (Limit::SQLITE_LIMIT_SQL_LENGTH, 10_000),
    (Limit::SQLITE_LIMIT_COLUMN, 64),
    (Limit::SQLITE_LIMIT_EXPR_DEPTH, 100),
    (Limit::SQLITE_LIMIT_ATTACHED, 0),
    (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 16),
    (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 1),
    (Limit::SQLITE_LIMIT_SCHEMA, schema::MAX_OBJECTS as i32),
    (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
];

/// Functions our statements may call, and those FTS5 and R*Tree call in
/// their own statements (`length` in their integrity checks, run by
/// `quick_check`). All are pure and cheap; none reads files.
const FUNCTIONS: [&str; 4] = ["count", "match", "bm25", "length"];

/// Why a file cannot be used, or an operation on it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    /// The file cannot be opened or read (missing, no permission). Cleaned.
    Io(String),
    /// The file is a symbolic link.
    Symlink,
    /// Not a regular file (a directory, a device, a FIFO).
    NotAFile,
    /// Not an SQLite database.
    NotSqlite,
    /// Shorter than its own header says: an interrupted copy?
    Truncated,
    /// The database is in WAL mode.
    WalMode,
    /// An impossible value in the database header.
    BadHeader(&'static str),
    /// The file was replaced while it was being opened.
    Changed,
    /// SQLite found the file damaged. Cleaned message.
    Corrupt(String),
    /// `quick_check` reported problems. Cleaned, bounded messages.
    Damaged(String),
    /// A statement ran past its budget.
    TooSlow,
    /// A value is larger than the limits allow.
    TooLarge,
    /// The authorizer refused a statement. Cleaned message.
    Denied(String),
    Schema(SchemaError),
    Metadata(MetadataError),
    Tile(TileError),
    /// `rs_coverage` holds invalid data.
    Coverage(String),
    /// A hardening setting did not take effect.
    Hardening(&'static str),
    /// Any other SQLite error. Cleaned message.
    Sqlite(String),
}

impl PackError {
    /// `false` only for I/O errors: the file could not be read at all.
    /// Everything else means "not a usable map pack".
    pub fn is_refusal(&self) -> bool {
        !matches!(self, PackError::Io(_))
    }
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::Io(m) => write!(f, "cannot read the file: {m}"),
            PackError::Symlink => write!(f, "the file is a symbolic link"),
            PackError::NotAFile => write!(f, "this is not a regular file"),
            PackError::NotSqlite => write!(f, "this is not a map pack (not an SQLite file)"),
            PackError::Truncated => {
                write!(f, "the file is incomplete (an interrupted copy?)")
            }
            PackError::WalMode => write!(
                f,
                "the file is in WAL mode, which cannot be read without writing beside it"
            ),
            PackError::BadHeader(w) => write!(f, "the file is damaged ({w})"),
            PackError::Changed => write!(f, "the file changed while it was being opened"),
            PackError::Corrupt(m) => write!(f, "the file is damaged ({m})"),
            PackError::Damaged(m) => write!(f, "the integrity check failed: {m}"),
            PackError::TooSlow => write!(f, "reading the file takes too long"),
            PackError::TooLarge => write!(f, "the file holds a value that is too large"),
            PackError::Denied(m) => write!(f, "the file asks for something not allowed ({m})"),
            PackError::Schema(e) => write!(f, "this is not a usable map pack: {e}"),
            PackError::Metadata(e) => write!(f, "this is not a usable map pack: {e}"),
            PackError::Tile(e) => e.fmt(f),
            PackError::Coverage(m) => write!(f, "the coverage area is invalid ({m})"),
            PackError::Hardening(w) => write!(f, "internal error: cannot set {w}"),
            PackError::Sqlite(m) => write!(f, "the file cannot be read ({m})"),
        }
    }
}

impl std::error::Error for PackError {}

impl From<SchemaError> for PackError {
    fn from(e: SchemaError) -> Self {
        PackError::Schema(e)
    }
}

impl From<MetadataError> for PackError {
    fn from(e: MetadataError) -> Self {
        PackError::Metadata(e)
    }
}

impl From<TileError> for PackError {
    fn from(e: TileError) -> Self {
        PackError::Tile(e)
    }
}

/// Map a rusqlite error, with SQLite's message cleaned (it can quote names
/// from the file).
fn sql_err(e: rusqlite::Error) -> PackError {
    let (code, extended, msg) = match &e {
        rusqlite::Error::SqliteFailure(f, msg) => (f.code, f.extended_code, msg.clone()),
        rusqlite::Error::SqlInputError { error, msg, .. } => {
            (error.code, error.extended_code, Some(msg.clone()))
        }
        _ => return PackError::Sqlite(clean_short(&e.to_string(), 200)),
    };
    let msg = clean_short(msg.as_deref().unwrap_or("unknown error"), 200);
    // SQLITE_LIMIT_SCHEMA reports a plain SQLITE_ERROR with this text.
    if msg == "too many schema objects" {
        return SchemaError::TooManyObjects.into();
    }
    match code {
        ErrorCode::OperationInterrupted => PackError::TooSlow,
        ErrorCode::DatabaseCorrupt => PackError::Corrupt(msg),
        ErrorCode::NotADatabase => PackError::NotSqlite,
        ErrorCode::TooBig => PackError::TooLarge,
        ErrorCode::AuthorizationForStatementDenied => PackError::Denied(msg),
        ErrorCode::CannotOpen if extended == rusqlite::ffi::SQLITE_CANTOPEN_SYMLINK => {
            PackError::Symlink
        }
        ErrorCode::CannotOpen => PackError::Io(msg),
        _ => PackError::Sqlite(msg),
    }
}

fn io_err(e: &std::io::Error) -> PackError {
    if e.raw_os_error() == Some(libc::ELOOP) {
        PackError::Symlink
    } else {
        PackError::Io(clean_short(&e.to_string(), 200))
    }
}

// ---------------------------------------------------------------------------
// Steps 1-3: the file
// ---------------------------------------------------------------------------

const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Check the 100-byte database header (`h` may be shorter if the file is)
/// against the file's length. <https://sqlite.org/fileformat.html#the_database_header>
pub fn check_header(h: &[u8], file_len: u64) -> Result<(), PackError> {
    if h.len() < 100 {
        let n = h.len().min(16);
        return Err(if n > 0 && h[..n] == MAGIC[..n] {
            PackError::Truncated
        } else {
            PackError::NotSqlite
        });
    }
    if h[..16] != MAGIC[..] {
        return Err(PackError::NotSqlite);
    }
    let page = match u16::from_be_bytes([h[16], h[17]]) {
        1 => 65_536u64,
        p if p >= 512 && p.is_power_of_two() => u64::from(p),
        _ => return Err(PackError::BadHeader("page size")),
    };
    match (h[18], h[19]) {
        (1, 1) => {}
        (w, r) if w == 2 || r == 2 => return Err(PackError::WalMode),
        _ => return Err(PackError::BadHeader("file format version")),
    }
    if h[21..24] != [64, 32, 32] {
        return Err(PackError::BadHeader("payload fractions"));
    }
    if file_len < page || !file_len.is_multiple_of(page) {
        return Err(PackError::Truncated);
    }
    // The page count in the header is valid when bytes 92-95 equal the
    // change counter (bytes 24-27), as SQLite itself decides.
    let pages = u32::from_be_bytes([h[28], h[29], h[30], h[31]]);
    if pages != 0 && h[24..28] == h[92..96] && u64::from(pages) * page > file_len {
        return Err(PackError::Truncated);
    }
    Ok(())
}

/// The directory part resolved, the last component kept as it is.
fn resolve(path: &Path) -> Result<PathBuf, PackError> {
    let name = path.file_name().ok_or(PackError::NotAFile)?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let dir = std::fs::canonicalize(parent).map_err(|e| io_err(&e))?;
    Ok(dir.join(name))
}

/// `file:` URI for SQLite, every byte outside the unreserved set
/// percent-encoded, read-only and immutable.
pub fn file_uri(path: &Path) -> String {
    let mut s = String::from("file:");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            s.push(char::from(b));
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s.push_str("?mode=ro&immutable=1");
    s
}

/// Open SQLite on `path` read-only, immutable, refusing symlinks.
fn open_connection(path: &Path) -> Result<Connection, PackError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NOFOLLOW
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE
        | OpenFlags::SQLITE_OPEN_EXRESCODE;
    Connection::open_with_flags(file_uri(path), flags).map_err(sql_err)
}

// ---------------------------------------------------------------------------
// Steps 4-7: the connection
// ---------------------------------------------------------------------------

/// The progress handler's state, shared with its closure.
struct BudgetState {
    base: Instant,
    /// Nanoseconds after `base` when the current operation must stop.
    deadline: AtomicU64,
    /// Handler calls left (each is `OPS_PER_CALL` operations).
    calls_left: AtomicU64,
}

impl BudgetState {
    fn arm(&self, b: Budget) {
        let now = self.base.elapsed().as_nanos();
        let deadline = now.saturating_add(b.time.as_nanos());
        self.deadline.store(
            u64::try_from(deadline).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.calls_left
            .store((b.ops / OPS_PER_CALL as u64).max(1), Ordering::Relaxed);
    }

    /// `true` to interrupt the running statement.
    fn exhausted(&self) -> bool {
        let left = self.calls_left.load(Ordering::Relaxed);
        if left == 0 {
            return true;
        }
        self.calls_left.store(left - 1, Ordering::Relaxed);
        let now = u64::try_from(self.base.elapsed().as_nanos()).unwrap_or(u64::MAX);
        now > self.deadline.load(Ordering::Relaxed)
    }
}

/// Tables the authorizer lets statements read, whatever the file.
const ALWAYS_READABLE: [&str; 11] = [
    "sqlite_schema",
    "sqlite_master",
    "metadata",
    "tiles",
    "map",
    "images",
    "tiles_shallow",
    "tiles_data",
    "pragma_table_xinfo",
    "pragma_index_list",
    "pragma_index_xinfo",
];

/// The authorizer: allow only what this reader needs. `extra` are the
/// `rs_` tables this file passed the checks for.
///
/// R*Tree prepares its write statements on its shadow tables when it
/// connects, even to read; preparing those is allowed for the verified
/// R*Tree tables only. They cannot run: the connection is read-only and
/// `query_only`, so any write fails at execution.
fn authorizer(
    extra: Vec<&'static str>,
) -> impl FnMut(AuthContext<'_>) -> Authorization + Send + 'static {
    let rtree = Family::PoiRtree.table_names();
    let rtree_ok = extra.iter().any(|t| rtree.contains(t));
    move |ctx: AuthContext<'_>| {
        // Nothing through a view or a trigger (both are disabled anyway).
        if ctx.accessor.is_some() {
            return Authorization::Deny;
        }
        let readable = |table: &str| {
            ALWAYS_READABLE
                .iter()
                .chain(extra.iter())
                .any(|t| t.eq_ignore_ascii_case(table))
        };
        // `count(*)` reports no database name.
        let main = matches!(ctx.database_name, None | Some("main"));
        let rtree_shadow = |table: &str| {
            rtree_ok && main && rtree[1..].iter().any(|t| t.eq_ignore_ascii_case(table))
        };
        let allowed = match ctx.action {
            AuthAction::Select => true,
            AuthAction::Read { table_name, .. } => main && readable(table_name),
            AuthAction::Insert { table_name }
            | AuthAction::Delete { table_name }
            | AuthAction::Update { table_name, .. } => rtree_shadow(table_name),
            AuthAction::Function { function_name } => FUNCTIONS
                .iter()
                .any(|f| f.eq_ignore_ascii_case(function_name)),
            AuthAction::Pragma {
                pragma_name,
                pragma_value,
            } => match pragma_name {
                // Read-only pragmas: the table-valued ones we use, and the
                // one FTS5 runs internally.
                "table_xinfo" | "index_list" | "index_xinfo" => true,
                "data_version" => pragma_value.is_none(),
                "quick_check" => pragma_value
                    .is_none_or(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())),
                _ => false,
            },
            _ => false,
        };
        if allowed {
            Authorization::Allow
        } else {
            Authorization::Deny
        }
    }
}

/// Apply steps 4-7 to a freshly opened connection and verify each.
fn harden(conn: &mut Connection) -> Result<Arc<BudgetState>, PackError> {
    // Step 4a: connection flags. These do not read the file.
    let flags = [
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true, "defensive mode"),
        (
            DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA,
            false,
            "trusted_schema",
        ),
        (
            DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
            false,
            "triggers off",
        ),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false, "views off"),
        (
            DbConfig::SQLITE_DBCONFIG_WRITABLE_SCHEMA,
            false,
            "writable_schema",
        ),
        (
            DbConfig::SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER,
            false,
            "fts3_tokenizer",
        ),
        (
            DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE,
            false,
            "attach create",
        ),
        (
            DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE,
            false,
            "attach write",
        ),
    ];
    for (option, on, what) in flags {
        conn.set_db_config(option, on)
            .map_err(|_| PackError::Hardening(what))?;
        if conn
            .db_config(option)
            .map_err(|_| PackError::Hardening(what))?
            != on
        {
            return Err(PackError::Hardening(what));
        }
    }
    // Step 5: limits.
    for (limit, value) in LIMITS {
        conn.set_limit(limit, value)
            .map_err(|_| PackError::Hardening("limits"))?;
        if conn
            .limit(limit)
            .map_err(|_| PackError::Hardening("limits"))?
            != value
        {
            return Err(PackError::Hardening("limits"));
        }
    }
    // Step 6: the progress handler, armed for loading the schema.
    let budget = Arc::new(BudgetState {
        base: Instant::now(),
        deadline: AtomicU64::new(0),
        calls_left: AtomicU64::new(0),
    });
    budget.arm(QUERY_BUDGET);
    let b = Arc::clone(&budget);
    conn.progress_handler(OPS_PER_CALL, Some(move || b.exhausted()))
        .map_err(|_| PackError::Hardening("progress handler"))?;
    // Step 4b: pragmas. Some load the schema: limits and handler are set.
    for (pragma, value) in [
        ("query_only", 1i64),
        ("trusted_schema", 0),
        ("cell_size_check", 1),
        ("mmap_size", 0),
        ("cache_size", -8192),
        ("temp_store", 2),
    ] {
        conn.pragma_update(None, pragma, value).map_err(sql_err)?;
        let now: i64 = conn
            .pragma_query_value(None, pragma, |r| r.get(0))
            .map_err(sql_err)?;
        if now != value {
            return Err(PackError::Hardening(pragma));
        }
    }
    // Step 7: the authorizer, last (the pragmas above are not allowed).
    conn.authorizer(Some(authorizer(Vec::new())))
        .map_err(|_| PackError::Hardening("authorizer"))?;
    Ok(budget)
}

// ---------------------------------------------------------------------------
// Step 8: the schema
// ---------------------------------------------------------------------------

fn text(v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Text(b) => Some(String::from_utf8_lossy(b).into_owned()),
        _ => None,
    }
}

fn read_schema(conn: &Connection) -> Result<Vec<SchemaRow>, PackError> {
    let mut st = conn
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema")
        .map_err(sql_err)?;
    let mut rows = st.query([]).map_err(sql_err)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(sql_err)? {
        if out.len() >= schema::MAX_OBJECTS {
            return Err(SchemaError::TooManyObjects.into());
        }
        let get = |i| r.get_ref(i).map(text).map_err(sql_err);
        out.push(SchemaRow {
            kind: get(0)?.unwrap_or_default(),
            name: get(1)?.unwrap_or_default(),
            table: get(2)?.unwrap_or_default(),
            sql: get(3)?,
        });
    }
    Ok(out)
}

/// The rows `sqlite_schema` holds for a pack made with [`profile::TABLES`]
/// (shadow tables included), computed once in a private in-memory database
/// with the same SQLite.
pub fn profile_schema() -> Result<&'static [SchemaRow], PackError> {
    static ROWS: OnceLock<Result<Vec<SchemaRow>, PackError>> = OnceLock::new();
    ROWS.get_or_init(|| {
        let conn = Connection::open_in_memory().map_err(sql_err)?;
        for sql in profile::TABLES {
            conn.execute_batch(sql).map_err(sql_err)?;
        }
        read_schema(&conn)
    })
    .as_ref()
    .map(Vec::as_slice)
    .map_err(Clone::clone)
}

/// Check the columns and indexes of the tables read (step 8).
fn check_tables(conn: &Connection, layout: TileLayout) -> Result<(), PackError> {
    let tables = std::iter::once(&schema::METADATA).chain(layout.tables());
    for &(table, required) in tables {
        let mut st = conn
            .prepare("SELECT name, hidden, dflt_value FROM pragma_table_xinfo(?1)")
            .map_err(sql_err)?;
        let columns = st
            .query_map([table], |r| {
                Ok(schema::Column {
                    name: r.get_ref(0).map(text)?.unwrap_or_default(),
                    hidden: r.get(1)?,
                    default: r.get_ref(2).map(text)?,
                })
            })
            .map_err(sql_err)?
            .take(schema::MAX_OBJECTS)
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        schema::check_columns(table, required, &columns)?;
        let mut st = conn
            .prepare("SELECT name, partial FROM pragma_index_list(?1)")
            .map_err(sql_err)?;
        let indexes = st
            .query_map([table], |r| {
                Ok((
                    r.get_ref(0).map(text)?.unwrap_or_default(),
                    r.get::<_, i64>(1)?,
                ))
            })
            .map_err(sql_err)?
            .take(schema::MAX_OBJECTS)
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        let mut xinfo = conn
            .prepare("SELECT cid FROM pragma_index_xinfo(?1)")
            .map_err(sql_err)?;
        for (index, partial) in indexes {
            if partial != 0 {
                return Err(SchemaError::ComputedIndex(table).into());
            }
            let cids = xinfo
                .query_map([&index], |r| r.get::<_, i64>(0))
                .map_err(sql_err)?
                .take(schema::MAX_OBJECTS)
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_err)?;
            // cid -2 is an expression; -1 the rowid.
            if cids.contains(&-2) {
                return Err(SchemaError::ComputedIndex(table).into());
            }
        }
    }
    Ok(())
}

/// Compile a fixed statement once, to learn at open time whether it works
/// on this schema.
fn compiles(conn: &Connection, sql: &str) -> Result<(), PackError> {
    match conn.prepare(sql) {
        Ok(_) => Ok(()),
        Err(e) => Err(match sql_err(e) {
            PackError::Sqlite(m) => SchemaError::Unusable(m).into(),
            other => other,
        }),
    }
}

fn read_metadata(conn: &Connection) -> Result<Metadata, PackError> {
    let mut st = conn
        .prepare("SELECT name, value FROM metadata")
        .map_err(sql_err)?;
    let mut rows = st.query([]).map_err(sql_err)?;
    let mut c = Collector::default();
    while let Some(r) = rows.next().map_err(sql_err)? {
        let key = match r.get_ref(0).map_err(sql_err)? {
            ValueRef::Text(b) => b,
            _ => b"",
        };
        let value = match r.get_ref(1).map_err(sql_err)? {
            ValueRef::Text(b) => RawValue::Text(b),
            ValueRef::Integer(i) => RawValue::Integer(i),
            ValueRef::Real(f) => RawValue::Real(f),
            ValueRef::Blob(_) => RawValue::Blob,
            ValueRef::Null => RawValue::Null,
        };
        c.push(key, value)?;
    }
    Ok(c.finish()?)
}

// ---------------------------------------------------------------------------
// The pack
// ---------------------------------------------------------------------------

/// Why a feature backed by `rs_` tables is not available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// The file is not a `resilience-1` pack.
    NotOurProfile,
    /// The tables are not there.
    Absent,
    /// The tables are there but differ from the profile at this object.
    Differs(String),
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unavailable::NotOurProfile => write!(f, "not a Resilience map pack"),
            Unavailable::Absent => write!(f, "not in this file"),
            Unavailable::Differs(n) => write!(f, "\"{n}\" differs from the expected table"),
        }
    }
}

/// `Ok(())` when available.
pub type Feature = Result<(), Unavailable>;

/// What the `rs_` tables of this file allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// The coverage polygon (`rs_coverage`).
    pub coverage: Feature,
    /// Place search (`rs_place` + `rs_place_fts`).
    pub search: Feature,
    /// Points of interest nearby (`rs_place` + `rs_poi_rtree`).
    pub nearby: Feature,
}

fn capabilities(
    rows: &[SchemaRow],
    profile: &Profile,
) -> Result<(Capabilities, Vec<&'static str>), PackError> {
    let canonical = profile_schema()?;
    let state = |f: Family| -> Feature {
        if *profile != Profile::Resilience1 {
            return Err(Unavailable::NotOurProfile);
        }
        match schema::compare_family(f, rows, canonical) {
            FamilyState::Exact => Ok(()),
            FamilyState::Absent => Err(Unavailable::Absent),
            FamilyState::Differs(n) => Err(Unavailable::Differs(n)),
        }
    };
    let both = |a: Feature, b: Feature| a.and(b);
    let caps = Capabilities {
        coverage: state(Family::Coverage),
        search: both(state(Family::Place), state(Family::PlaceFts)),
        nearby: both(state(Family::Place), state(Family::PoiRtree)),
    };
    let readable = Family::ALL
        .into_iter()
        .filter(|f| state(*f).is_ok())
        .flat_map(|f| f.table_names().iter().copied())
        .collect();
    Ok((caps, readable))
}

/// Tile numbers in either convention.
pub trait TileKey: Copy {
    fn tms(self) -> TmsTile;
}

impl TileKey for TmsTile {
    fn tms(self) -> TmsTile {
        self
    }
}

impl TileKey for TileId {
    fn tms(self) -> TmsTile {
        self.to_tms()
    }
}

/// One ring of the coverage area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ring {
    pub hole: bool,
    pub points: Vec<Micro>,
}

/// The area a pack's data covers (see `profile.rs`: even-odd rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub rings: Vec<Ring>,
}

impl Coverage {
    pub fn vertex_count(&self) -> usize {
        self.rings.iter().map(|r| r.points.len()).sum()
    }
}

/// An open map pack. Every method bounds its own work; a hostile file can
/// make them fail, never hang or write.
pub struct Pack {
    conn: Connection,
    /// Kept open: the file whose identity was checked.
    _file: File,
    path: PathBuf,
    size: u64,
    budget: Arc<BudgetState>,
    metadata: Metadata,
    layout: TileLayout,
    capabilities: Capabilities,
}

impl std::fmt::Debug for Pack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pack")
            .field("path", &self.path)
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl Pack {
    /// Open and check a map pack. See the module documentation.
    pub fn open(path: &Path) -> Result<Pack, PackError> {
        // Step 1.
        let path = resolve(path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(&path)
            .map_err(|e| io_err(&e))?;
        let st = file.metadata().map_err(|e| io_err(&e))?;
        if !st.file_type().is_file() {
            return Err(PackError::NotAFile);
        }
        // Step 2.
        let mut header = Vec::with_capacity(100);
        (&mut file)
            .take(100)
            .read_to_end(&mut header)
            .map_err(|e| io_err(&e))?;
        check_header(&header, st.len())?;
        // Step 3.
        let mut conn = open_connection(&path)?;
        let now = std::fs::symlink_metadata(&path).map_err(|e| io_err(&e))?;
        if now.dev() != st.dev() || now.ino() != st.ino() || !now.file_type().is_file() {
            return Err(PackError::Changed);
        }
        // Steps 4-7.
        let budget = harden(&mut conn)?;
        // Step 8.
        budget.arm(QUERY_BUDGET);
        let rows = read_schema(&conn)?;
        let layout = schema::classify(&rows)?;
        check_tables(&conn, layout)?;
        compiles(&conn, layout.tile_sql())?;
        compiles(&conn, layout.count_sql())?;
        let metadata = read_metadata(&conn)?;
        let (capabilities, readable) = capabilities(&rows, &metadata.profile)?;
        // The authorizer now also lets the verified rs_ tables be read.
        conn.authorizer(Some(authorizer(readable)))
            .map_err(|_| PackError::Hardening("authorizer"))?;
        Ok(Pack {
            conn,
            _file: file,
            path,
            size: st.len(),
            budget,
            metadata,
            layout,
            capabilities,
        })
    }

    /// The path opened (directory part resolved).
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_size(&self) -> u64 {
        self.size
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn layout(&self) -> TileLayout {
        self.layout
    }

    /// The decompressed bytes of a tile (an MVT), or `None` if the pack
    /// has no such tile (including zooms outside its range).
    pub fn tile_bytes(&self, tile: impl TileKey) -> Result<Option<Vec<u8>>, PackError> {
        self.tile_bytes_within(tile, TILE_BUDGET)
    }

    fn tile_bytes_within(
        &self,
        tile: impl TileKey,
        budget: Budget,
    ) -> Result<Option<Vec<u8>>, PackError> {
        let t = tile.tms();
        let zoom = self.metadata.zoom;
        if t.zoom() < zoom.min() || t.zoom() > zoom.max() {
            return Ok(None);
        }
        self.budget.arm(budget);
        let mut st = self.conn.prepare(self.layout.tile_sql()).map_err(sql_err)?;
        let mut rows = st.query((t.zoom(), t.column(), t.row())).map_err(sql_err)?;
        let Some(row) = rows.next().map_err(|e| match sql_err(e) {
            PackError::TooLarge => PackError::Tile(TileError::TooLargeCompressed),
            other => other,
        })?
        else {
            return Ok(None);
        };
        let blob = match row.get_ref(0).map_err(sql_err)? {
            ValueRef::Blob(b) | ValueRef::Text(b) => b,
            ValueRef::Null => return Ok(None),
            ValueRef::Integer(_) | ValueRef::Real(_) => {
                return Err(TileError::UnknownEncoding.into());
            }
        };
        Ok(Some(inflate::decompress_tile(blob)?))
    }

    /// A decoded tile, or `None` if the pack has no such tile.
    pub fn tile(&self, tile: impl TileKey) -> Result<Option<mvt::Tile>, PackError> {
        match self.tile_bytes(tile)? {
            None => Ok(None),
            Some(bytes) => mvt::decode(&bytes)
                .map(Some)
                .map_err(|e| TileError::Mvt(e).into()),
        }
    }

    /// The coverage area, or `None` if the file has no verified
    /// `rs_coverage` (see [`Capabilities::coverage`]).
    pub fn coverage(&self) -> Result<Option<Coverage>, PackError> {
        if self.capabilities.coverage.is_err() {
            return Ok(None);
        }
        self.budget.arm(QUERY_BUDGET);
        let mut st = self
            .conn
            .prepare("SELECT hole, points FROM rs_coverage ORDER BY ring")
            .map_err(sql_err)?;
        let mut rows = st.query([]).map_err(sql_err)?;
        let mut rings = Vec::new();
        let mut total = 0usize;
        while let Some(r) = rows.next().map_err(sql_err)? {
            if rings.len() >= profile::MAX_COVERAGE_RINGS {
                return Err(PackError::Coverage("too many rings".into()));
            }
            let hole = match r.get_ref(0).map_err(sql_err)? {
                ValueRef::Integer(0) => false,
                ValueRef::Integer(1) => true,
                _ => return Err(PackError::Coverage("hole flag".into())),
            };
            let points = match r.get_ref(1).map_err(sql_err)? {
                ValueRef::Blob(b) => {
                    profile::decode_ring(b).map_err(|e| PackError::Coverage(e.to_string()))?
                }
                _ => return Err(PackError::Coverage("points are not a blob".into())),
            };
            total += points.len();
            if total > profile::MAX_COVERAGE_VERTICES {
                return Err(PackError::Coverage("too many points".into()));
            }
            rings.push(Ring { hole, points });
        }
        if rings.is_empty() {
            return Err(PackError::Coverage("no rings".into()));
        }
        Ok(Some(Coverage { rings }))
    }

    /// The number of tiles, or `None` if counting would take more than
    /// [`QUERY_BUDGET`] (a large pack on a slow stick).
    pub fn tile_count(&self) -> Result<Option<u64>, PackError> {
        self.budget.arm(QUERY_BUDGET);
        match self
            .conn
            .query_row(self.layout.count_sql(), [], |r| r.get::<_, i64>(0))
        {
            Ok(n) => Ok(Some(u64::try_from(n).unwrap_or(0))),
            Err(e) => match sql_err(e) {
                PackError::TooSlow => Ok(None),
                other => Err(other),
            },
        }
    }

    /// `PRAGMA quick_check`, bounded by [`QUICK_CHECK_BUDGET`]. `Ok` if the
    /// file is sound; [`PackError::Damaged`] with SQLite's first messages
    /// otherwise; [`PackError::TooSlow`] if it could not finish in time.
    pub fn quick_check(&self) -> Result<(), PackError> {
        self.quick_check_within(QUICK_CHECK_BUDGET)
    }

    fn quick_check_within(&self, budget: Budget) -> Result<(), PackError> {
        self.budget.arm(budget);
        let mut st = self
            .conn
            .prepare("PRAGMA quick_check(10)")
            .map_err(sql_err)?;
        let messages = st
            .query_map([], |r| Ok(r.get_ref(0).map(text)?.unwrap_or_default()))
            .map_err(sql_err)?
            .take(10)
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        if messages == ["ok"] {
            Ok(())
        } else {
            Err(PackError::Damaged(clean_short(&messages.join("; "), 300)))
        }
    }

    /// Whether ADR §7 wants `quick_check` run before use when the file was
    /// not checked against the catalog.
    pub fn quick_check_recommended(&self) -> bool {
        self.size <= QUICK_CHECK_AUTO_MAX
    }
}

#[cfg(test)]
mod tests;
