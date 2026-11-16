//! Validation of a map pack's `metadata` table (MBTiles 1.3 keys plus the
//! `resilience_` keys of ADR 0016 §1). Pure: the reader passes the rows.
//!
//! Policy:
//! - every value we keep is bounded in bytes (over the bound: refused, as
//!   no real pack comes near) and then [`clean`]ed, so it can be printed;
//! - a key we use that appears twice is refused (which one would be true?);
//! - keys we do not use are ignored, whatever their size;
//! - numbers (zooms, bounds, centre) must parse and lie within the world,
//!   otherwise the file is refused rather than drawn somewhere arbitrary;
//! - `format` must be `pbf`: raster packs are not supported (ADR §1);
//! - a file that says it is `resilience-1` must carry the fields the
//!   viewer has to show (data date, source, coverage, licence,
//!   attribution).

use resilience_core::maps::geo::{LatLon, MAX_TILE_ZOOM, ZoomRange};
use resilience_core::packs::valid_data_date;

use crate::profile;
use crate::text::clean;

/// Most rows read from `metadata`.
pub const MAX_ROWS: usize = 1000;

/// The keys we read, each with its largest accepted value in bytes.
pub const KEYS: [(&str, usize); 14] = [
    ("name", 256),
    ("format", 16),
    ("minzoom", 8),
    ("maxzoom", 8),
    ("bounds", 128),
    ("center", 128),
    ("attribution", 1024),
    ("description", 2048),
    ("resilience_profile", 16),
    ("resilience_data_date", 32),
    ("resilience_source", 1024),
    ("resilience_coverage", 2048),
    ("resilience_license", 256),
    ("resilience_builder", 256),
];

/// What a pack says about itself, validated. Every string is printable.
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    pub name: String,
    pub zoom: ZoomRange,
    /// The area holding tiles; the whole world if the file does not say.
    pub bounds: Bounds,
    /// Whether `bounds` came from the file.
    pub bounds_given: bool,
    pub center: Option<Center>,
    pub attribution: Option<String>,
    pub description: Option<String>,
    pub profile: Profile,
    /// `YYYY-MM-DD` or `YYYY-MM`.
    pub data_date: Option<String>,
    pub source: Option<String>,
    pub coverage: Option<String>,
    pub license: Option<String>,
    pub builder: Option<String>,
}

/// West, south, east, north in degrees; west < east, south < north.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl Bounds {
    pub const WORLD: Bounds = Bounds {
        west: -180.0,
        south: -85.051_128_779_806_59,
        east: 180.0,
        north: 85.051_128_779_806_59,
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Center {
    pub at: LatLon,
    pub zoom: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Profile {
    /// Our own packs: `resilience_profile` = 1.
    Resilience1,
    /// Another `resilience_profile` value (a newer profile); cleaned.
    Unknown(String),
    /// No `resilience_profile`: a file from another tool.
    None,
}

/// Why the metadata was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataError {
    TooManyRows,
    Missing(&'static str),
    Duplicate(&'static str),
    TooLong(&'static str),
    NotText(&'static str),
    Invalid(&'static str, &'static str),
    /// `format` is not `pbf`: the (cleaned) value.
    UnsupportedFormat(String),
}

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetadataError::TooManyRows => {
                write!(f, "the metadata table has more than {MAX_ROWS} entries")
            }
            MetadataError::Missing(k) => write!(f, "the metadata has no {k}"),
            MetadataError::Duplicate(k) => write!(f, "the metadata gives {k} twice"),
            MetadataError::TooLong(k) => write!(f, "the metadata's {k} is too long"),
            MetadataError::NotText(k) => write!(f, "the metadata's {k} is not text"),
            MetadataError::Invalid(k, why) => write!(f, "the metadata's {k} is invalid: {why}"),
            MetadataError::UnsupportedFormat(v) => write!(
                f,
                "the tiles are in format \"{v}\": only vector tiles (pbf) can be shown"
            ),
        }
    }
}

impl std::error::Error for MetadataError {}

/// One raw value as SQLite gave it (text borrowed, not yet bounded).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RawValue<'a> {
    Text(&'a [u8]),
    /// Some tools store zooms as numbers.
    Integer(i64),
    Real(f64),
    /// A blob, which no key we read may be.
    Blob,
    Null,
}

/// Collects the rows of interest while the reader steps through the table.
#[derive(Debug, Default)]
pub struct Collector {
    values: [Option<String>; KEYS.len()],
    rows: usize,
}

impl Collector {
    /// Take one row. `key` and the value are as stored (not yet bounded).
    pub fn push(&mut self, key: &[u8], value: RawValue<'_>) -> Result<(), MetadataError> {
        self.rows += 1;
        if self.rows > MAX_ROWS {
            return Err(MetadataError::TooManyRows);
        }
        let Some(i) = KEYS.iter().position(|(k, _)| k.as_bytes() == key) else {
            return Ok(());
        };
        let (k, max) = KEYS[i];
        let v = match value {
            RawValue::Null => return Ok(()),
            RawValue::Blob => return Err(MetadataError::NotText(k)),
            RawValue::Integer(i) => i.to_string(),
            RawValue::Real(f) => f.to_string(),
            RawValue::Text(b) if b.len() > max => return Err(MetadataError::TooLong(k)),
            RawValue::Text(b) => String::from_utf8_lossy(b).into_owned(),
        };
        if self.values[i].is_some() {
            return Err(MetadataError::Duplicate(k));
        }
        self.values[i] = Some(clean(&v));
        Ok(())
    }

    fn take(&mut self, key: &str) -> Option<String> {
        let i = KEYS.iter().position(|(k, _)| *k == key)?;
        self.values[i].take().filter(|v| !v.is_empty())
    }

    /// Validate what was collected.
    pub fn finish(mut self) -> Result<Metadata, MetadataError> {
        let name = self.take("name").ok_or(MetadataError::Missing("name"))?;
        let format = self
            .take("format")
            .ok_or(MetadataError::Missing("format"))?;
        if format != "pbf" {
            return Err(MetadataError::UnsupportedFormat(format));
        }
        let min = zoom(self.take("minzoom"), "minzoom")?;
        let max = zoom(self.take("maxzoom"), "maxzoom")?;
        let zoom = ZoomRange::new(min, max)
            .ok_or(MetadataError::Invalid("zoom range", "minzoom > maxzoom"))?;
        let (bounds, bounds_given) = match self.take("bounds") {
            None => (Bounds::WORLD, false),
            Some(b) => (parse_bounds(&b)?, true),
        };
        let center = self.take("center").map(|c| parse_center(&c)).transpose()?;
        let profile = match self.take("resilience_profile") {
            None => Profile::None,
            Some(p) if p == profile::PROFILE_VALUE => Profile::Resilience1,
            Some(p) => Profile::Unknown(p),
        };
        let data_date = self.take("resilience_data_date");
        if let Some(d) = &data_date
            && !valid_data_date(d)
        {
            return Err(MetadataError::Invalid(
                "resilience_data_date",
                "not YYYY-MM-DD or YYYY-MM",
            ));
        }
        let m = Metadata {
            name,
            zoom,
            bounds,
            bounds_given,
            center,
            attribution: self.take("attribution"),
            description: self.take("description"),
            profile,
            data_date,
            source: self.take("resilience_source"),
            coverage: self.take("resilience_coverage"),
            license: self.take("resilience_license"),
            builder: self.take("resilience_builder"),
        };
        if m.profile == Profile::Resilience1 {
            for (k, v) in [
                ("resilience_data_date", &m.data_date),
                ("resilience_source", &m.source),
                ("resilience_coverage", &m.coverage),
                ("resilience_license", &m.license),
                ("attribution", &m.attribution),
            ] {
                if v.is_none() {
                    return Err(MetadataError::Missing(k));
                }
            }
        }
        Ok(m)
    }
}

fn zoom(v: Option<String>, key: &'static str) -> Result<u8, MetadataError> {
    let v = v.ok_or(MetadataError::Missing(key))?;
    if v.is_empty() || v.len() > 2 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err(MetadataError::Invalid(key, "not a whole number"));
    }
    v.parse::<u8>()
        .ok()
        .filter(|z| *z <= MAX_TILE_ZOOM)
        .ok_or(MetadataError::Invalid(key, "above 24"))
}

fn numbers<const N: usize>(s: &str, key: &'static str) -> Result<[f64; N], MetadataError> {
    let mut out = [0.0; N];
    let mut parts = s.split(',');
    for slot in &mut out {
        *slot = parts
            .next()
            .and_then(|p| p.trim().parse::<f64>().ok())
            .filter(|f| f.is_finite())
            .ok_or(MetadataError::Invalid(key, "not a list of numbers"))?;
    }
    if parts.next().is_some() {
        return Err(MetadataError::Invalid(key, "too many numbers"));
    }
    Ok(out)
}

fn parse_bounds(s: &str) -> Result<Bounds, MetadataError> {
    let [west, south, east, north] = numbers::<4>(s, "bounds")?;
    let inside = |lon: f64, lat: f64| lon.abs() <= 180.0 && lat.abs() <= 90.0;
    if !inside(west, south) || !inside(east, north) {
        return Err(MetadataError::Invalid("bounds", "outside the world"));
    }
    if west >= east || south >= north {
        return Err(MetadataError::Invalid(
            "bounds",
            "west/east or south/north reversed",
        ));
    }
    Ok(Bounds {
        west,
        south,
        east,
        north,
    })
}

fn parse_center(s: &str) -> Result<Center, MetadataError> {
    let (lon, lat, zoom) = match s.split(',').count() {
        2 => {
            let [lon, lat] = numbers::<2>(s, "center")?;
            (lon, lat, None)
        }
        3 => {
            let [lon, lat, z] = numbers::<3>(s, "center")?;
            if z.fract() != 0.0 || !(0.0..=f64::from(MAX_TILE_ZOOM)).contains(&z) {
                return Err(MetadataError::Invalid("center", "zoom not 0-24"));
            }
            (lon, lat, Some(z as u8))
        }
        _ => return Err(MetadataError::Invalid("center", "not lon,lat[,zoom]")),
    };
    let at = LatLon::new(lat, lon).ok_or(MetadataError::Invalid("center", "outside the world"))?;
    Ok(Center { at, zoom })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(rows: &[(&str, &str)]) -> Result<Metadata, MetadataError> {
        let mut c = Collector::default();
        for (k, v) in rows {
            c.push(k.as_bytes(), RawValue::Text(v.as_bytes()))?;
        }
        c.finish()
    }

    const BASE: [(&str, &str); 4] = [
        ("name", "Test"),
        ("format", "pbf"),
        ("minzoom", "0"),
        ("maxzoom", "14"),
    ];

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut v = BASE.to_vec();
        v.extend_from_slice(extra);
        v
    }

    #[test]
    fn a_minimal_third_party_file_is_accepted() {
        let m = collect(&BASE).expect("should succeed");
        assert_eq!(m.name, "Test");
        assert_eq!((m.zoom.min(), m.zoom.max()), (0, 14));
        assert_eq!(m.bounds, Bounds::WORLD);
        assert!(!m.bounds_given);
        assert_eq!(m.profile, Profile::None);
    }

    #[test]
    fn resilience_packs_need_their_fields() {
        let full = with(&[
            ("resilience_profile", "1"),
            ("resilience_data_date", "2026-01-01"),
            ("resilience_source", "luxembourg-260101.osm.pbf sha256:ab"),
            ("resilience_coverage", "Luxembourg"),
            ("resilience_license", "ODbL-1.0"),
            ("attribution", "(c) OpenStreetMap contributors"),
            ("bounds", "5.73,49.44,6.53,50.19"),
            ("center", "6.13,49.61,10"),
        ]);
        let m = collect(&full).expect("should succeed");
        assert_eq!(m.profile, Profile::Resilience1);
        assert_eq!(m.bounds.east, 6.53);
        assert_eq!(m.center.expect("should succeed").zoom, Some(10));
        for missing in [
            "resilience_data_date",
            "resilience_source",
            "resilience_coverage",
            "resilience_license",
            "attribution",
        ] {
            let rows: Vec<_> = full
                .iter()
                .copied()
                .filter(|(k, _)| *k != missing)
                .collect();
            assert_eq!(collect(&rows), Err(MetadataError::Missing(missing)));
        }
        let other = with(&[("resilience_profile", "2")]);
        assert_eq!(
            collect(&other).expect("should succeed").profile,
            Profile::Unknown("2".into())
        );
    }

    #[test]
    fn missing_and_absurd_values_are_refused() {
        use MetadataError::*;
        for (rows, err) in [
            (
                vec![("format", "pbf"), ("minzoom", "0"), ("maxzoom", "1")],
                Missing("name"),
            ),
            (
                vec![("name", "x"), ("minzoom", "0"), ("maxzoom", "1")],
                Missing("format"),
            ),
            (with(&[("name", "again")]), Duplicate("name")),
            (BASE[..3].to_vec(), Missing("maxzoom")),
        ] {
            assert_eq!(collect(&rows), Err(err));
        }
        let invalid = |extra: &[(&'static str, &'static str)]| match collect(&with(extra)) {
            Err(Invalid(k, _)) => k,
            other => panic!("{extra:?}: {other:?}"),
        };
        assert_eq!(invalid(&[("bounds", "-181,0,10,10")]), "bounds");
        assert_eq!(invalid(&[("bounds", "0,-91,10,10")]), "bounds");
        assert_eq!(invalid(&[("bounds", "10,0,5,10")]), "bounds");
        assert_eq!(invalid(&[("bounds", "0,0,10")]), "bounds");
        assert_eq!(invalid(&[("bounds", "0,0,10,10,1")]), "bounds");
        assert_eq!(invalid(&[("bounds", "0,0,nan,10")]), "bounds");
        assert_eq!(invalid(&[("bounds", "0,0,inf,10")]), "bounds");
        assert_eq!(invalid(&[("center", "0,95")]), "center");
        assert_eq!(invalid(&[("center", "0,0,25")]), "center");
        assert_eq!(invalid(&[("center", "0,0,1.5")]), "center");
        assert_eq!(
            invalid(&[("resilience_data_date", "January")]),
            "resilience_data_date"
        );
        for (min, max) in [
            ("14", "0"),
            ("0", "25"),
            ("-1", "3"),
            ("0", "1e1"),
            ("", "3"),
        ] {
            let rows = [
                ("name", "x"),
                ("format", "pbf"),
                ("minzoom", min),
                ("maxzoom", max),
            ];
            assert!(
                matches!(collect(&rows), Err(Invalid(..) | Missing(_))),
                "{min} {max}"
            );
        }
        assert_eq!(
            collect(&[
                ("name", "x"),
                ("format", "png"),
                ("minzoom", "0"),
                ("maxzoom", "1")
            ]),
            Err(UnsupportedFormat("png".into()))
        );
    }

    #[test]
    fn long_values_are_refused_and_unknown_keys_ignored() {
        let mut c = Collector::default();
        c.push(b"json", RawValue::Text("x".repeat(3_000_000).as_bytes()))
            .expect("should succeed");
        c.push(b"\x1b[2J", RawValue::Blob).expect("should succeed");
        assert_eq!(
            c.push(b"name", RawValue::Text("n".repeat(1 << 20).as_bytes())),
            Err(MetadataError::TooLong("name"))
        );
        let mut c = Collector::default();
        assert_eq!(
            c.push(b"name", RawValue::Blob),
            Err(MetadataError::NotText("name"))
        );
        let mut c = Collector::default();
        for (k, v) in [
            (&b"name"[..], RawValue::Text(b"n")),
            (b"format", RawValue::Text(b"pbf")),
        ] {
            c.push(k, v).expect("should succeed");
        }
        c.push(b"minzoom", RawValue::Integer(2))
            .expect("should succeed");
        c.push(b"maxzoom", RawValue::Real(9.0))
            .expect("should succeed");
        assert_eq!(c.finish().map(|m| (m.zoom.min(), m.zoom.max())), Ok((2, 9)));
        let mut c = Collector::default();
        for _ in 0..MAX_ROWS {
            c.push(b"other", RawValue::Null).expect("should succeed");
        }
        assert_eq!(
            c.push(b"other", RawValue::Null),
            Err(MetadataError::TooManyRows)
        );
    }

    #[test]
    fn strings_are_cleaned() {
        let m = collect(&[
            ("name", "Evil\x1b]0;owned\x07\nname"),
            ("format", "pbf"),
            ("minzoom", "0"),
            ("maxzoom", "1"),
            ("attribution", "\u{202E}tfel"),
        ])
        .expect("should succeed");
        assert_eq!(m.name, "Evil?]0;owned??name");
        assert_eq!(m.attribution.as_deref(), Some("?tfel"));
    }
}
