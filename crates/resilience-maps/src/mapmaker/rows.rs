//! The side file between `classify` and `finish`: one JSON object per
//! line, each a search or POI row before `near` is known and before
//! street segments are merged. Positions are already microdegrees, so
//! `finish` parses no floating point.
//!
//! ```json
//! {"kind":"street","merge":true,"class":"residential","name":"Rue de l'Essai","lon":6001000,"lat":49001000,"rank":6,"bbox":[6000000,49000500,6002000,49001500]}
//! ```
//!
//! `merge` rows are segments (streets, rivers): `finish` keeps one row
//! per kind, name and nearest settlement, with the union of their boxes.

use serde::{Deserialize, Serialize};

use crate::profile::{self, Micro};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Settlement,
    Street,
    Poi,
    Natural,
}

impl Kind {
    /// The value of `rs_place.kind` (one of `profile::PLACE_KINDS`).
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Settlement => profile::PLACE_KINDS[0],
            Kind::Street => profile::PLACE_KINDS[1],
            Kind::Poi => profile::PLACE_KINDS[2],
            Kind::Natural => profile::PLACE_KINDS[3],
        }
    }

    /// Order of the kinds in `rs_place` (ADR §4: settlements, streets,
    /// POIs, then natural features).
    pub fn order(self) -> i64 {
        match self {
            Kind::Settlement => 0,
            Kind::Street => 1,
            Kind::Poi => 2,
            Kind::Natural => 3,
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Settlement, Kind::Street, Kind::Poi, Kind::Natural]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

/// Longest `class` accepted in the side file.
pub const MAX_CLASS: usize = 64;
/// Longest `name` accepted in the side file, in bytes.
pub const MAX_NAME_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merge: bool,
    pub class: String,
    /// Empty for unnamed points of interest and peaks (listed by
    /// "Nearby", never found by search).
    pub name: String,
    pub lon: i32,
    pub lat: i32,
    pub rank: i64,
    /// West, south, east, north.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[i32; 4]>,
}

impl Row {
    pub fn at(&self) -> Micro {
        Micro {
            lon: self.lon,
            lat: self.lat,
        }
    }

    /// Check a row read back from the side file.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.class.is_empty() || self.class.len() > MAX_CLASS {
            return Err("bad class");
        }
        if self.name.len() > MAX_NAME_BYTES {
            return Err("name too long");
        }
        if Micro::new(self.lon, self.lat).is_none() {
            return Err("position outside the world");
        }
        if let Some([w, s, e, n]) = self.bbox
            && (Micro::new(w, s).is_none() || Micro::new(e, n).is_none() || w > e || s > n)
        {
            return Err("bad bounding box");
        }
        if self.merge && (self.bbox.is_none() || self.name.is_empty()) {
            return Err("a segment needs a name and a box");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_round_trip_and_are_checked() {
        let r = Row {
            kind: Kind::Street,
            merge: true,
            class: "residential".into(),
            name: "Rue de l'Essai".into(),
            lon: 6_001_000,
            lat: 49_001_000,
            rank: 6,
            bbox: Some([6_000_000, 49_000_500, 6_002_000, 49_001_500]),
        };
        let s = serde_json::to_string(&r).expect("json");
        assert_eq!(
            s,
            r#"{"kind":"street","merge":true,"class":"residential","name":"Rue de l'Essai","lon":6001000,"lat":49001000,"rank":6,"bbox":[6000000,49000500,6002000,49001500]}"#
        );
        assert_eq!(serde_json::from_str::<Row>(&s).expect("row"), r);
        assert_eq!(r.check(), Ok(()));
        let p: Row = serde_json::from_str(
            r#"{"kind":"poi","class":"pharmacy","name":"","lon":1,"lat":2,"rank":0}"#,
        )
        .expect("row");
        assert!(!p.merge && p.bbox.is_none());
        assert_eq!(p.check(), Ok(()));
        let bad = |f: &dyn Fn(&mut Row)| {
            let mut x = r.clone();
            f(&mut x);
            x.check()
        };
        assert!(bad(&|x| x.lat = 90_000_001).is_err());
        assert!(bad(&|x| x.bbox = Some([2, 0, 1, 0])).is_err());
        assert!(bad(&|x| x.bbox = None).is_err());
        assert!(bad(&|x| x.class = String::new()).is_err());
        assert!(
            serde_json::from_str::<Row>(
                r#"{"kind":"road","class":"x","name":"","lon":0,"lat":0,"rank":0}"#
            )
            .is_err()
        );
        for k in [Kind::Settlement, Kind::Street, Kind::Poi, Kind::Natural] {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
        }
    }
}
