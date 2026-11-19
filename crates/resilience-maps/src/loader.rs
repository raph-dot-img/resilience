//! The map reader: opens packs, runs their integrity check and reads tiles,
//! on a thread of its own so that a slow stick or a crafted file never
//! freezes the interface (ADR 0016 §3, §7).
//!
//! The viewer's state machine (`app.rs`) never calls the reader: it queues
//! [`Request`]s, and `main.rs` (or a test) carries them to a [`Worker`] and
//! the [`Response`]s back. In the program the worker runs on a thread
//! ([`spawn`]); tests call [`Worker::handle`] directly, which makes them
//! deterministic.
//!
//! Tile requests come as a whole list per view ([`Request::Tiles`]); a newer
//! list replaces the tiles of an older one not read yet, so panning
//! quickly does not queue reads for views already left.
//!
//! **Checked or not** ([`Catalog`]): the reader decides it from the system's
//! pack catalog (`/etc/resilience/packs.toml`, root-owned), not from the
//! caller. A file counts as checked when it is directly in the packs
//! directory (only the pack store writes there, and it renames a file into
//! place only after its SHA-256 matched the catalog; older versions were
//! checked when they were installed) or is one of the built-in packs (in
//! the read-only Nix store). Anything else, such as a file on a stick, is
//! "not checked", even if its name and size match a catalog entry. Without
//! a readable catalog nothing is checked. The comparison uses the path the
//! reader opened, whose directory part is resolved.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use resilience_core::maps::geo::TileId;
use resilience_core::maps::{mvt, search};
use resilience_core::packs::PackConfig;

use crate::cache::{self, Key};
use crate::metadata::Metadata;
use crate::pack::{Capabilities, Coverage, Pack, PackError};

/// What the system's catalog says is checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    dir: Option<PathBuf>,
    builtins: Vec<PathBuf>,
}

impl Catalog {
    /// Nothing is checked.
    pub fn none() -> Catalog {
        Catalog::default()
    }

    pub fn from_config(cfg: &PackConfig) -> Catalog {
        Catalog {
            dir: std::fs::canonicalize(&cfg.dir).ok(),
            builtins: cfg
                .builtin
                .iter()
                .filter_map(|b| std::fs::canonicalize(&b.path).ok())
                .collect(),
        }
    }

    /// The catalog at `path`, or [`Catalog::none`] if it cannot be read.
    pub fn load(path: &Path) -> Catalog {
        PackConfig::load(path).map_or_else(|_| Catalog::none(), |c| Catalog::from_config(&c))
    }

    /// Whether the file the reader opened at `opened` (directory part
    /// resolved) was checked against the catalog.
    pub fn is_checked(&self, opened: &Path) -> bool {
        if self.builtins.iter().any(|b| b == opened) {
            return true;
        }
        let in_dir = match (&self.dir, opened.parent()) {
            (Some(d), Some(p)) => d == p,
            _ => false,
        };
        let hidden = opened
            .file_name()
            .is_none_or(|n| n.to_string_lossy().starts_with('.'));
        in_dir && !hidden
    }
}

/// Integrity of an open pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Integrity {
    /// Installed or built in: its hash matched the catalog.
    Catalog,
    /// Not in the catalog; `quick_check` is running.
    Checking,
    /// Not in the catalog; `quick_check` passed.
    Passed,
    /// Not in the catalog and not checked (too large for the automatic
    /// check, which the viewer offers).
    NotChecked,
    /// The check found damage (cleaned message). The pack is not used.
    Damaged(String),
}

/// What the viewer keeps about an open pack.
#[derive(Debug, Clone)]
pub struct Summary {
    /// The path opened (directory part resolved).
    pub path: PathBuf,
    pub size: u64,
    pub metadata: Metadata,
    pub capabilities: Capabilities,
    /// The coverage polygon, if the file has a valid one.
    pub coverage: Option<Coverage>,
    /// Why a coverage table that exists could not be used.
    pub coverage_problem: Option<String>,
    pub integrity: Integrity,
}

#[derive(Debug, Clone)]
pub enum Request {
    Open {
        index: usize,
        path: PathBuf,
    },
    /// Run `quick_check` on an open pack (offered for large files).
    Check {
        index: usize,
    },
    /// The tiles the current view needs, most important first.
    Tiles {
        generation: u64,
        tiles: Vec<Key>,
    },
    /// Search every open pack that can be searched for `fts`, an FTS5
    /// expression from `search::fts_query` (never user text).
    Search {
        generation: u64,
        fts: String,
    },
}

/// The outcome of one tile read.
#[derive(Debug, Clone)]
pub enum TileResult {
    /// Decoded, with its estimated size in memory.
    Tile(Arc<mvt::Tile>, usize),
    /// The pack has no such tile.
    Absent,
    /// Damaged: gzip, MVT or SQLite error (cleaned message).
    Damaged(String),
    /// The read ran past its time budget.
    Slow,
}

#[derive(Debug, Clone)]
pub enum Response {
    Opened {
        index: usize,
        summary: Box<Summary>,
    },
    /// The file cannot be used. `refusal` is false when it could not be
    /// read at all (missing, permission).
    Refused {
        index: usize,
        message: String,
        refusal: bool,
    },
    Checking {
        index: usize,
    },
    Checked {
        index: usize,
        result: Result<(), String>,
    },
    Tile {
        generation: u64,
        key: Key,
        result: TileResult,
        elapsed: Duration,
    },
    /// The places found, with the packs searched and those that failed.
    Found {
        generation: u64,
        hits: Vec<search::Hit>,
        searched: Vec<usize>,
        failed: Vec<(usize, String)>,
    },
}

/// The reader's state: the open packs, by the viewer's index.
pub struct Worker {
    catalog: Catalog,
    packs: Vec<Option<Pack>>,
    /// In the program: integrity checks run on threads of their own (each
    /// with its own connection to the file) and answer here, so that the
    /// other packs are read and drawn while a large file is checked. In
    /// tests (`None`) a check runs inline.
    checks: Option<Sender<Response>>,
}

impl Worker {
    pub fn new(catalog: Catalog) -> Worker {
        Worker {
            catalog,
            packs: Vec::new(),
            checks: None,
        }
    }

    fn slot(&mut self, index: usize) -> &mut Option<Pack> {
        if self.packs.len() <= index {
            self.packs.resize_with(index + 1, || None);
        }
        &mut self.packs[index]
    }

    /// Handle one request completely, sending each response as it comes.
    pub fn handle(&mut self, req: Request, send: &mut dyn FnMut(Response)) {
        match req {
            Request::Open { index, path } => self.open(index, &path, send),
            Request::Check { index } => self.check(index, send),
            Request::Tiles { generation, tiles } => {
                for key in tiles {
                    send(self.tile(generation, key));
                }
            }
            Request::Search { generation, fts } => send(self.search(generation, &fts)),
        }
    }

    /// [`Worker::handle`] collecting the responses (for tests).
    pub fn handle_all(&mut self, reqs: impl IntoIterator<Item = Request>) -> Vec<Response> {
        let mut out = Vec::new();
        for r in reqs {
            self.handle(r, &mut |resp| out.push(resp));
        }
        out
    }

    fn open(&mut self, index: usize, path: &Path, send: &mut dyn FnMut(Response)) {
        let pack = match Pack::open(path) {
            Ok(p) => p,
            Err(e) => {
                send(Response::Refused {
                    index,
                    message: e.to_string(),
                    refusal: e.is_refusal(),
                });
                return;
            }
        };
        let (coverage, coverage_problem) = match pack.coverage() {
            Ok(c) => (c, None),
            Err(e) => (None, Some(e.to_string())),
        };
        let checked = self.catalog.is_checked(pack.path());
        let auto = !checked && pack.quick_check_recommended();
        let integrity = if checked {
            Integrity::Catalog
        } else if auto {
            Integrity::Checking
        } else {
            Integrity::NotChecked
        };
        send(Response::Opened {
            index,
            summary: Box::new(Summary {
                path: pack.path().to_path_buf(),
                size: pack.file_size(),
                metadata: pack.metadata().clone(),
                capabilities: pack.capabilities().clone(),
                coverage,
                coverage_problem,
                integrity,
            }),
        });
        *self.slot(index) = Some(pack);
        if auto {
            self.check(index, send);
        }
    }

    fn check(&mut self, index: usize, send: &mut dyn FnMut(Response)) {
        let Some(pack) = self.packs.get(index).and_then(Option::as_ref) else {
            return;
        };
        send(Response::Checking { index });
        if let Some(tx) = &self.checks {
            let path = pack.path().to_path_buf();
            let tx = tx.clone();
            let spawned = std::thread::Builder::new()
                .name("map-check".into())
                .spawn(move || {
                    let result = Pack::open(&path)
                        .and_then(|p| p.quick_check())
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Response::Checked { index, result });
                });
            if spawned.is_ok() {
                // The pack is not drawn until the viewer has the result.
                return;
            }
        }
        let result = pack.quick_check().map_err(|e| e.to_string());
        if result.is_err() {
            *self.slot(index) = None;
        }
        send(Response::Checked { index, result });
    }

    /// Search every open pack that has the search tables.
    pub fn search(&mut self, generation: u64, fts: &str) -> Response {
        let (mut hits, mut searched, mut failed) = (Vec::new(), Vec::new(), Vec::new());
        for (index, pack) in self.packs.iter().enumerate() {
            let Some(pack) = pack else { continue };
            if pack.capabilities().search.is_err() {
                continue;
            }
            match pack.search(fts) {
                Ok(found) => {
                    searched.push(index);
                    hits.extend(found.into_iter().map(|mut h| {
                        h.pack = index;
                        h
                    }));
                }
                Err(e) => failed.push((index, e.to_string())),
            }
        }
        Response::Found {
            generation,
            hits,
            searched,
            failed,
        }
    }

    /// Read and decode one tile.
    pub fn tile(&mut self, generation: u64, key: Key) -> Response {
        let start = Instant::now();
        let result = match self.slot(key.pack).as_ref() {
            None => TileResult::Absent,
            Some(pack) => read_tile(pack, key.tile),
        };
        Response::Tile {
            generation,
            key,
            result,
            elapsed: start.elapsed(),
        }
    }
}

fn read_tile(pack: &Pack, id: TileId) -> TileResult {
    match pack.tile_bytes(id) {
        Ok(None) => TileResult::Absent,
        Ok(Some(bytes)) => match mvt::decode(&bytes) {
            Ok(t) => {
                let est = cache::estimate(&t, bytes.len());
                TileResult::Tile(Arc::new(t), est)
            }
            Err(e) => TileResult::Damaged(format!("the map data is damaged ({e})")),
        },
        Err(PackError::TooSlow) => TileResult::Slow,
        Err(e) => TileResult::Damaged(e.to_string()),
    }
}

/// The reader thread has ended (it panicked): nothing more can be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderGone;

/// The viewer's end of a reader thread.
pub struct Channel {
    pub requests: Sender<Request>,
    pub responses: Receiver<Response>,
}

impl Channel {
    /// Responses ready now; `Err` once the reader thread has ended.
    pub fn ready(&self) -> Result<Vec<Response>, ReaderGone> {
        let mut out = Vec::new();
        loop {
            match self.responses.try_recv() {
                Ok(r) => out.push(r),
                Err(TryRecvError::Empty) => return Ok(out),
                Err(TryRecvError::Disconnected) => {
                    return if out.is_empty() {
                        Err(ReaderGone)
                    } else {
                        Ok(out)
                    };
                }
            }
        }
    }
}

/// Start a reader thread. It ends when the viewer drops its [`Channel`].
pub fn spawn(mut worker: Worker) -> std::io::Result<Channel> {
    let (req_tx, req_rx) = mpsc::channel::<Request>();
    let (resp_tx, resp_rx) = mpsc::channel::<Response>();
    worker.checks = Some(resp_tx.clone());
    std::thread::Builder::new()
        .name("map-reader".into())
        .spawn(move || serve(&mut worker, &req_rx, &resp_tx))?;
    Ok(Channel {
        requests: req_tx,
        responses: resp_rx,
    })
}

fn serve(worker: &mut Worker, rx: &Receiver<Request>, tx: &Sender<Response>) {
    let mut pending: VecDeque<Key> = VecDeque::new();
    let mut generation = 0;
    let alive = std::cell::Cell::new(true);
    let mut send = |r: Response| {
        if tx.send(r).is_err() {
            alive.set(false);
        }
    };
    loop {
        let mut incoming = Vec::new();
        if pending.is_empty() {
            match rx.recv() {
                Ok(r) => incoming.push(r),
                Err(_) => return,
            }
        }
        loop {
            match rx.try_recv() {
                Ok(r) => incoming.push(r),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for r in incoming {
            match r {
                Request::Tiles {
                    generation: g,
                    tiles,
                } => {
                    generation = g;
                    pending = tiles.into();
                }
                other => worker.handle(other, &mut send),
            }
        }
        if let Some(key) = pending.pop_front() {
            send(worker.tile(generation, key));
        }
        if !alive.get() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{TempDir, write_pack};

    #[test]
    fn checked_only_in_the_packs_directory_or_built_in() {
        let dir = TempDir::new("catalog");
        let packs = dir.file("packs");
        std::fs::create_dir_all(packs.join(".staging")).expect("mkdir");
        let builtin = dir.file("store-world.mbtiles");
        std::fs::write(&builtin, b"x").expect("write");
        let cat = Catalog {
            dir: std::fs::canonicalize(&packs).ok(),
            builtins: vec![std::fs::canonicalize(&builtin).expect("canon")],
        };
        let canon = std::fs::canonicalize(dir.path()).expect("canon");
        assert!(cat.is_checked(&canon.join("packs/map-osm-lux_2026-01.mbtiles")));
        assert!(cat.is_checked(&canon.join("store-world.mbtiles")));
        assert!(!cat.is_checked(&canon.join("packs/.staging/map.mbtiles.part")));
        assert!(!cat.is_checked(&canon.join("packs/.hidden.mbtiles")));
        assert!(!cat.is_checked(&canon.join("stick/map-osm-lux_2026-01.mbtiles")));
        assert!(!cat.is_checked(&canon.join("packs")));
        assert!(!Catalog::none().is_checked(&canon.join("packs/map.mbtiles")));
        // A catalog that cannot be read checks nothing.
        assert_eq!(Catalog::load(&dir.file("missing.toml")), Catalog::none());
    }

    #[test]
    fn a_catalog_file_is_read() {
        let dir = TempDir::new("catalog-file");
        let packs = dir.file("packs");
        std::fs::create_dir_all(&packs).expect("mkdir");
        let world = dir.file("world.mbtiles");
        write_pack(&world);
        let toml = format!(
            "dir = \"{}\"\nstore_user = \"resilience-packs\"\nstore = \"/s\"\npkexec = \"/p\"\n\
             pkttyagent = \"/a\"\ncurl = \"/c\"\nsha256sum = \"/h\"\nkiwix_serve = \"/k\"\n\
             browser = [\"/b\"]\nsession_shell = \"/sh\"\n\n[[builtin]]\ntitle = \"World\"\n\
             description = \"d\"\ncategory = \"maps\"\nlanguage = \"en\"\npath = \"{}\"\n\
             license = \"CC0-1.0\"\nsource = \"Natural Earth\"\n",
            packs.display(),
            world.display()
        );
        let cfg = dir.file("packs.toml");
        std::fs::write(&cfg, toml).expect("write");
        let cat = Catalog::load(&cfg);
        assert!(cat.dir.is_some() && cat.builtins.len() == 1, "{cat:?}");
        let mut w = Worker::new(cat);
        let out = w.handle_all([Request::Open {
            index: 0,
            path: world.clone(),
        }]);
        let Some(Response::Opened { summary, .. }) = out.first() else {
            panic!("{out:?}")
        };
        assert_eq!(summary.integrity, Integrity::Catalog);
        assert_eq!(out.len(), 1, "no check for a checked pack");
    }

    #[test]
    fn an_unchecked_small_pack_is_checked_before_use() {
        let dir = TempDir::new("loader-check");
        let p = dir.file("map.mbtiles");
        write_pack(&p);
        let mut w = Worker::new(Catalog::none());
        let out = w.handle_all([Request::Open { index: 2, path: p }]);
        assert!(
            matches!(&out[0], Response::Opened { index: 2, summary } if summary.integrity == Integrity::Checking)
        );
        assert!(matches!(out[1], Response::Checking { index: 2 }));
        assert!(matches!(
            &out[2],
            Response::Checked {
                index: 2,
                result: Ok(())
            }
        ));
        let tile = TileId::new(0, 0, 0).expect("tile");
        let r = w.tile(7, Key { pack: 2, tile });
        assert!(
            matches!(
                r,
                Response::Tile {
                    generation: 7,
                    result: TileResult::Tile(..),
                    ..
                }
            ),
            "{r:?}"
        );
        let r = w.tile(7, Key { pack: 5, tile });
        assert!(
            matches!(
                r,
                Response::Tile {
                    result: TileResult::Absent,
                    ..
                }
            ),
            "unknown pack"
        );
    }

    #[test]
    fn the_thread_answers_and_ends() {
        let dir = TempDir::new("loader-thread");
        let p = dir.file("map.mbtiles");
        write_pack(&p);
        let ch = spawn(Worker::new(Catalog::none())).expect("spawn");
        ch.requests
            .send(Request::Open { index: 0, path: p })
            .expect("send");
        let tile = TileId::new(1, 0, 1).expect("tile");
        ch.requests
            .send(Request::Tiles {
                generation: 1,
                tiles: vec![Key { pack: 0, tile }],
            })
            .expect("send");
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while got.len() < 4 && Instant::now() < deadline {
            got.extend(ch.ready().expect("alive"));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            got.iter().any(|r| matches!(
                r,
                Response::Tile {
                    result: TileResult::Tile(..),
                    ..
                }
            )),
            "{got:?}"
        );
        // The check ran on its own thread and answered too.
        assert!(
            got.iter().any(|r| matches!(
                r,
                Response::Checked {
                    index: 0,
                    result: Ok(())
                }
            )),
            "{got:?}"
        );
        drop(ch);
    }
}
