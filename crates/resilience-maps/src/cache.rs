//! The viewer's cache of decoded tiles (ADR 0016 §3): least recently used,
//! bounded both in count ([`MAX_TILES`]) and in an estimate of bytes
//! ([`MAX_BYTES`]).
//!
//! Two small side tables keep the answers that are not tiles, so that the
//! viewer does not ask again on every frame:
//! - tiles a pack does not have (common: overzoom outside the data);
//! - tiles that failed to read or decode ("bad"), each with the reason, so
//!   a damaged tile is reported once and never retried. A tile that was
//!   only too slow to read is retried after [`SLOW_RETRY`] (a USB stick
//!   can be slow for a moment).
//!
//! Tiles the current view needs are *pinned*: inserting a new tile never
//! evicts them. When the pinned tiles alone exceed the byte bound, the new
//! tile is not kept ([`Insert::TooMuch`]) and the viewer says the place has
//! too much data to show at once.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use resilience_core::maps::geo::TileId;
use resilience_core::maps::mvt;

/// Decoded tiles kept at most.
pub const MAX_TILES: usize = 64;
/// Estimated bytes of decoded tiles kept at most.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Entries of each side table (absent and bad tiles).
pub const MAX_SIDE: usize = 4096;
/// A tile that was too slow to read is asked for again after this.
pub const SLOW_RETRY: Duration = Duration::from_secs(10);

/// A tile of one open pack (index in the viewer's pack list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub pack: usize,
    pub tile: TileId,
}

/// What the cache knows about a key.
#[derive(Debug, Clone)]
pub enum Lookup {
    Tile(Arc<mvt::Tile>),
    /// The pack has no such tile.
    Absent,
    /// It could not be read or decoded.
    Bad,
    /// Not known: ask the reader.
    Miss,
}

/// Why a tile is bad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadKind {
    /// Damaged (gzip, MVT, SQLite error): never retried.
    Damaged,
    /// Too slow to read: retried after [`SLOW_RETRY`].
    Slow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insert {
    Kept,
    /// Not kept: the pinned tiles already use the byte budget.
    TooMuch,
}

struct Entry {
    tile: Arc<mvt::Tile>,
    bytes: usize,
    used: u64,
}

/// Bounded first-in first-out set.
struct Side<V> {
    map: HashMap<Key, V>,
    order: VecDeque<Key>,
}

impl<V> Side<V> {
    fn new() -> Self {
        Side {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, k: Key, v: V) {
        if self.map.insert(k, v).is_none() {
            self.order.push_back(k);
        }
        while self.map.len() > MAX_SIDE {
            match self.order.pop_front() {
                Some(old) => {
                    self.map.remove(&old);
                }
                None => break,
            }
        }
    }

    fn remove(&mut self, k: &Key) {
        if self.map.remove(k).is_some() {
            self.order.retain(|o| o != k);
        }
    }
}

pub struct TileCache {
    tiles: HashMap<Key, Entry>,
    absent: Side<()>,
    bad: Side<(BadKind, Instant)>,
    bytes: usize,
    clock: u64,
    max_tiles: usize,
    max_bytes: usize,
}

impl Default for TileCache {
    fn default() -> Self {
        TileCache::new(MAX_TILES, MAX_BYTES)
    }
}

/// Estimated memory of a decoded tile: its decompressed size (strings,
/// keys, values) plus 8 bytes per vertex and 64 per feature.
pub fn estimate(tile: &mvt::Tile, decompressed: usize) -> usize {
    let mut n = decompressed;
    for l in tile.layers() {
        n = n
            .saturating_add(l.vertex_count().saturating_mul(8))
            .saturating_add(l.len().saturating_mul(64));
    }
    n.max(1)
}

impl TileCache {
    pub fn new(max_tiles: usize, max_bytes: usize) -> TileCache {
        TileCache {
            tiles: HashMap::new(),
            absent: Side::new(),
            bad: Side::new(),
            bytes: 0,
            clock: 0,
            max_tiles: max_tiles.max(1),
            max_bytes: max_bytes.max(1),
        }
    }

    /// Look a key up; a hit counts as a use.
    pub fn get(&mut self, k: &Key, now: Instant) -> Lookup {
        if let Some(e) = self.tiles.get_mut(k) {
            self.clock += 1;
            e.used = self.clock;
            return Lookup::Tile(Arc::clone(&e.tile));
        }
        if self.absent.map.contains_key(k) {
            return Lookup::Absent;
        }
        match self.bad.map.get(k) {
            Some((BadKind::Slow, at)) if now.duration_since(*at) >= SLOW_RETRY => {
                self.bad.remove(k);
                Lookup::Miss
            }
            Some(_) => Lookup::Bad,
            None => Lookup::Miss,
        }
    }

    /// Keep a decoded tile, evicting the least recently used tiles that
    /// are not in `pinned`.
    pub fn insert(
        &mut self,
        k: Key,
        tile: Arc<mvt::Tile>,
        bytes: usize,
        pinned: &HashSet<Key>,
    ) -> Insert {
        if let Some(old) = self.tiles.remove(&k) {
            self.bytes -= old.bytes;
        }
        while self.tiles.len() >= self.max_tiles
            || self.bytes.saturating_add(bytes) > self.max_bytes
        {
            let victim = self
                .tiles
                .iter()
                .filter(|(key, _)| !pinned.contains(key))
                .min_by_key(|(_, e)| e.used)
                .map(|(key, _)| *key);
            match victim {
                Some(v) => {
                    if let Some(e) = self.tiles.remove(&v) {
                        self.bytes -= e.bytes;
                    }
                }
                None => return Insert::TooMuch,
            }
        }
        self.clock += 1;
        self.bytes += bytes;
        self.tiles.insert(
            k,
            Entry {
                tile,
                bytes,
                used: self.clock,
            },
        );
        Insert::Kept
    }

    pub fn insert_absent(&mut self, k: Key) {
        self.absent.insert(k, ());
    }

    pub fn insert_bad(&mut self, k: Key, kind: BadKind, now: Instant) {
        self.bad.insert(k, (kind, now));
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn bad_count(&self) -> usize {
        self.bad.map.len()
    }

    /// Forget everything about one pack (it was closed or found damaged).
    pub fn forget_pack(&mut self, pack: usize) {
        let keys: Vec<Key> = self
            .tiles
            .keys()
            .filter(|k| k.pack == pack)
            .copied()
            .collect();
        for k in keys {
            if let Some(e) = self.tiles.remove(&k) {
                self.bytes -= e.bytes;
            }
        }
        self.absent.map.retain(|k, _| k.pack != pack);
        self.absent.order.retain(|k| k.pack != pack);
        self.bad.map.retain(|k, _| k.pack != pack);
        self.bad.order.retain(|k| k.pack != pack);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pack: usize, x: u32) -> Key {
        Key {
            pack,
            tile: TileId::new(14, x, 0).expect("tile"),
        }
    }

    fn tile() -> Arc<mvt::Tile> {
        Arc::new(mvt::Tile::default())
    }

    #[test]
    fn bounded_in_count_least_recently_used_first() {
        let now = Instant::now();
        let mut c = TileCache::new(3, 1000);
        let none = HashSet::new();
        for x in 0..3 {
            assert_eq!(c.insert(key(0, x), tile(), 10, &none), Insert::Kept);
        }
        // Use 0, so 1 is the least recently used.
        assert!(matches!(c.get(&key(0, 0), now), Lookup::Tile(_)));
        c.insert(key(0, 3), tile(), 10, &none);
        assert_eq!(c.len(), 3);
        assert!(matches!(c.get(&key(0, 1), now), Lookup::Miss));
        assert!(matches!(c.get(&key(0, 0), now), Lookup::Tile(_)));
        assert!(matches!(c.get(&key(0, 3), now), Lookup::Tile(_)));
        assert_eq!(c.bytes(), 30);
        // Re-inserting a key does not count it twice.
        c.insert(key(0, 3), tile(), 20, &none);
        assert_eq!((c.len(), c.bytes()), (3, 40));
    }

    #[test]
    fn bounded_in_bytes_and_pinned_tiles_stay() {
        let mut c = TileCache::new(64, 100);
        let pinned: HashSet<Key> = [key(0, 0), key(0, 1)].into_iter().collect();
        c.insert(key(0, 0), tile(), 40, &pinned);
        c.insert(key(0, 1), tile(), 40, &pinned);
        c.insert(key(0, 2), tile(), 20, &pinned);
        assert_eq!(c.bytes(), 100);
        // The unpinned one goes first.
        assert_eq!(c.insert(key(0, 3), tile(), 20, &pinned), Insert::Kept);
        assert!(matches!(c.get(&key(0, 2), Instant::now()), Lookup::Miss));
        // Pinned tiles are never evicted: a tile that cannot fit is refused.
        let all: HashSet<Key> = (0..5).map(|x| key(0, x)).collect();
        assert_eq!(c.insert(key(0, 4), tile(), 30, &all), Insert::TooMuch);
        assert!(c.bytes() <= 100 && c.len() == 3);
    }

    #[test]
    fn absent_and_bad_tiles_are_remembered() {
        let t0 = Instant::now();
        let mut c = TileCache::default();
        c.insert_absent(key(1, 0));
        c.insert_bad(key(1, 1), BadKind::Damaged, t0);
        c.insert_bad(key(1, 2), BadKind::Slow, t0);
        assert!(matches!(c.get(&key(1, 0), t0), Lookup::Absent));
        assert!(matches!(c.get(&key(1, 1), t0), Lookup::Bad));
        assert!(matches!(c.get(&key(1, 2), t0), Lookup::Bad));
        let later = t0 + SLOW_RETRY;
        assert!(
            matches!(c.get(&key(1, 1), later), Lookup::Bad),
            "never retried"
        );
        assert!(
            matches!(c.get(&key(1, 2), later), Lookup::Miss),
            "slow: retried"
        );
        // The side tables are bounded too.
        for x in 0..(MAX_SIDE as u32 + 10) {
            c.insert_absent(key(2, x));
        }
        assert!(c.absent.map.len() <= MAX_SIDE && c.absent.order.len() <= MAX_SIDE);
        c.forget_pack(1);
        assert!(matches!(c.get(&key(1, 1), t0), Lookup::Miss));
        assert_eq!(c.bad_count(), 0);
    }
}
