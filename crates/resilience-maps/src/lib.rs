//! Offline map viewer of Resilience OS (ADR 0016): the hardened reader for
//! map packs (MBTiles files with vector tiles and our own `rs_` tables).
//!
//! Map files are untrusted input, parsed as the logged-in user in this
//! separate foreground process (never in the TUI):
//!
//! - [`pack`]: opening one file with every SQLite hardening of ADR §7,
//!   tiles, coverage and the capabilities of its `rs_` tables;
//! - [`schema`]: what the reader accepts in a file's schema;
//! - [`metadata`]: validation of the `metadata` table;
//! - [`profile`]: the exact schema of profile `resilience-1` and its blob
//!   layouts, shared with the pack builder;
//! - [`inflate`]: tile decompression with hard limits;
//! - [`rlimit`]: the process's address-space limit;
//! - [`info`]: the `info` command's output;
//! - [`text`]: cleaning untrusted strings before they reach a terminal;
//! - [`mapmaker`]: the pack builder `resilience-mapmaker` (build machines
//!   only).

pub mod inflate;
pub mod info;
pub mod mapmaker;
pub mod metadata;
pub mod pack;
pub mod profile;
pub mod rlimit;
pub mod schema;
pub mod text;

// The test fixtures name this crate `resilience_maps`, so that
// `tests/cli.rs` can include the same file.
#[cfg(test)]
extern crate self as resilience_maps;
#[cfg(test)]
mod fixtures;
