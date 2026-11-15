//! Offline maps (ADR 0016): pure code shared by the map viewer and its
//! tests. Nothing here touches files, SQLite or the terminal.
//!
//! - [`geo`]: Web Mercator, the view model and zoom ladder, tiles covering
//!   a view, tile-to-screen transforms, sphere distance and direction,
//!   coordinate formatting and parsing, scale bars.

pub mod geo;
