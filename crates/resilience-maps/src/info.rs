//! Output of `resilience-maps info FILE`: one `key: value` per line, in a
//! fixed order, for the TUI and tests to parse and people to read. Keys are
//! lower-case ASCII with hyphens; values are single lines whose characters
//! were cleaned ([`crate::text::clean`]), so nothing from the file can
//! reach the terminal as a control character or start a new line.
//!
//! ```text
//! file: /run/media/ann/STICK/map-osm-luxembourg_2026-01.mbtiles
//! name: Luxembourg
//! profile: resilience-1           (or "none", or "unknown VALUE")
//! data-date: 2026-01-01           (or "unknown")
//! source: luxembourg-260101.osm.pbf sha256:...   (or "unknown")
//! coverage: Luxembourg, whole country             (or "not stated")
//! license: ODbL-1.0                                (or "not stated")
//! attribution: (c) OpenStreetMap contributors      (or "not stated")
//! builder: resilience-mapmaker 0.1.0               (or "unknown")
//! format: pbf
//! zoom: 0-14
//! bounds: 5.73590,49.44700,6.53080,50.18280   (west,south,east,north;
//!                                              "whole world (not stated)")
//! center: 49.61160,6.13190,10                 (lat,lon[,zoom] or "none")
//! size: 35512320
//! tiles: 1903                                 (or "not counted")
//! coverage-area: 1 rings, 1234 points         (or "not available: REASON")
//! search: available                           (or "not available: REASON")
//! points-of-interest: available               (or "not available: REASON")
//! integrity: not checked                      ("ok", "damaged: ...")
//! ```

use std::fmt::Write;

use crate::metadata::Profile;
use crate::pack::{Pack, PackError};
use crate::text::clean;

fn or<'a>(v: &'a Option<String>, missing: &'a str) -> &'a str {
    v.as_deref().unwrap_or(missing)
}

fn feature(f: &crate::pack::Feature) -> String {
    match f {
        Ok(()) => "available".into(),
        Err(why) => format!("not available: {why}"),
    }
}

/// The `info` text for an open pack. `check` is the result of
/// [`Pack::quick_check`] if it was run.
pub fn render(pack: &Pack, check: Option<&Result<(), PackError>>) -> String {
    let m = pack.metadata();
    let mut out = String::new();
    let mut line = |k: &str, v: &str| {
        // `v` is clean already (metadata) or ours; clean again anyway.
        let _ = writeln!(out, "{k}: {}", clean(v));
    };
    line("file", &pack.path().to_string_lossy());
    line("name", &m.name);
    let profile = match &m.profile {
        Profile::Resilience1 => crate::profile::PROFILE_NAME.to_string(),
        Profile::Unknown(v) => format!("unknown {v}"),
        Profile::None => "none".into(),
    };
    line("profile", &profile);
    line("data-date", or(&m.data_date, "unknown"));
    line("source", or(&m.source, "unknown"));
    line("coverage", or(&m.coverage, "not stated"));
    line("license", or(&m.license, "not stated"));
    line("attribution", or(&m.attribution, "not stated"));
    line("builder", or(&m.builder, "unknown"));
    line("format", "pbf");
    line("zoom", &format!("{}-{}", m.zoom.min(), m.zoom.max()));
    let b = m.bounds;
    let bounds = format!("{:.5},{:.5},{:.5},{:.5}", b.west, b.south, b.east, b.north);
    if m.bounds_given {
        line("bounds", &bounds);
    } else {
        line("bounds", &format!("{bounds} (whole world, not stated)"));
    }
    let center = match m.center {
        None => "none".to_string(),
        Some(c) => {
            let mut s = format!("{:.5},{:.5}", c.at.lat(), c.at.lon());
            if let Some(z) = c.zoom {
                let _ = write!(s, ",{z}");
            }
            s
        }
    };
    line("center", &center);
    line("size", &pack.file_size().to_string());
    let tiles = match pack.tile_count() {
        Ok(Some(n)) => n.to_string(),
        Ok(None) => "not counted (too many to count quickly)".into(),
        Err(e) => format!("not counted: {e}"),
    };
    line("tiles", &tiles);
    let coverage = match pack.coverage() {
        Ok(Some(c)) => format!("{} rings, {} points", c.rings.len(), c.vertex_count()),
        Ok(None) => feature(&pack.capabilities().coverage),
        Err(e) => format!("not available: {e}"),
    };
    line("coverage-area", &coverage);
    line("search", &feature(&pack.capabilities().search));
    line("points-of-interest", &feature(&pack.capabilities().nearby));
    let integrity = match check {
        None => "not checked".to_string(),
        Some(Ok(())) => "ok".into(),
        Some(Err(PackError::Damaged(m))) => format!("damaged: {m}"),
        Some(Err(e)) => format!("not completed: {e}"),
    };
    line("integrity", &integrity);
    out
}
