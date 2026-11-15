//! Offline maps (ADR 0016): pure code shared by the map viewer and its
//! tests. Nothing here touches files, SQLite or the terminal.
//!
//! - [`geo`]: Web Mercator, the view model and zoom ladder, tiles covering
//!   a view, tile-to-screen transforms, sphere distance and direction,
//!   coordinate formatting and parsing, scale bars.
//! - [`mvt`]: the bounded Mapbox Vector Tile decoder.
//! - [`canvas`], [`style`], [`label`]: the pixel canvas, the
//!   `resilience-1` style table and label placement.

pub mod canvas;
pub mod style;
pub mod geo;
pub mod mvt;
#[cfg(any(test, feature = "test-support"))]
pub mod mvt_encode;
