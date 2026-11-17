//! From OSM tags to the layers and classes of `resilience-1` (ADR 0016 §1,
//! §4, §5), as tables that can be read next to the ADR.
//!
//! The class names written into tiles are never spelt here: every table
//! maps OSM values to the enums of `resilience_core::maps::style`, whose
//! `as_str` is the contract with the renderer. The minimum zoom of each
//! class comes from the style table too ([`min_zoom`]).
//!
//! OSM tags are free text typed by volunteers: every value is bounded
//! ([`MAX_TAG_BYTES`]), names are cleaned and cut ([`clean_name`]),
//! numbers are parsed defensively ([`parse_ele`], [`parse_population`]),
//! and anything that does not fit is left out rather than guessed.

use std::borrow::Cow;

use resilience_core::maps::style::{
    self, AdminLevel, Class, LandClass, PeakClass, PlaceClass, PoiCategory, RailClass, RoadClass,
    WaterClass,
};

/// Tag values longer than this are treated as absent (OSM's own limit is
/// 255 characters, so only broken or hostile data is affected).
pub const MAX_TAG_BYTES: usize = 1024;
/// Names are cut to this many characters.
pub const MAX_NAME_CHARS: usize = 120;
/// `ref` values are cut to this many characters.
pub const MAX_REF_CHARS: usize = 24;
/// An OSM value used as a POI class: at most this long, `[a-z0-9_]` only.
pub const MAX_CLASS_BYTES: usize = 32;

/// Read access to one object's tags.
pub trait Tags {
    /// The value of `key`, if present, a string and at most
    /// [`MAX_TAG_BYTES`] long.
    fn tag(&self, key: &str) -> Option<&str>;

    fn is(&self, key: &str, value: &str) -> bool {
        self.tag(key) == Some(value)
    }
}

impl Tags for serde_json::Map<String, serde_json::Value> {
    fn tag(&self, key: &str) -> Option<&str> {
        self.get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|v| v.len() <= MAX_TAG_BYTES)
    }
}

impl Tags for [(&str, &str)] {
    fn tag(&self, key: &str) -> Option<&str> {
        self.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .filter(|v| v.len() <= MAX_TAG_BYTES)
    }
}

/// The geometry osmium gave an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geom {
    /// A node.
    Point,
    /// A way exported as a line string.
    Line,
    /// A closed way or a multipolygon/boundary relation exported as an area.
    Area,
}

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

/// `highway=*` -> `roads` class (lines only). Links take their road's
/// class; `living_street` and `pedestrian` are residential; footways,
/// cycleways, bridleways and steps are paths; `road` (not yet classified)
/// is unclassified. Anything else (construction, proposed, platform,
/// raceway, corridor...) is left out.
pub const ROADS: &[(&str, RoadClass)] = &[
    ("motorway", RoadClass::Motorway),
    ("motorway_link", RoadClass::Motorway),
    ("trunk", RoadClass::Trunk),
    ("trunk_link", RoadClass::Trunk),
    ("primary", RoadClass::Primary),
    ("primary_link", RoadClass::Primary),
    ("secondary", RoadClass::Secondary),
    ("secondary_link", RoadClass::Secondary),
    ("tertiary", RoadClass::Tertiary),
    ("tertiary_link", RoadClass::Tertiary),
    ("unclassified", RoadClass::Unclassified),
    ("road", RoadClass::Unclassified),
    ("residential", RoadClass::Residential),
    ("living_street", RoadClass::Residential),
    ("pedestrian", RoadClass::Residential),
    ("service", RoadClass::Service),
    ("track", RoadClass::Track),
    ("path", RoadClass::Path),
    ("footway", RoadClass::Path),
    ("cycleway", RoadClass::Path),
    ("bridleway", RoadClass::Path),
    ("steps", RoadClass::Path),
];

/// `railway=*` -> `rail` class (lines only). Subways (mostly underground)
/// and disused or abandoned lines are left out, and so are tracks with
/// `service` in [`RAIL_SERVICE_LEFT_OUT`] (yards and sidings: clutter at
/// console resolution).
pub const RAIL: &[(&str, RailClass)] = &[
    ("rail", RailClass::Rail),
    ("narrow_gauge", RailClass::Rail),
    ("preserved", RailClass::Rail),
    ("light_rail", RailClass::LightRail),
    ("tram", RailClass::LightRail),
];
pub const RAIL_SERVICE_LEFT_OUT: &[&str] = &["yard", "siding", "spur", "crossover"];

/// `waterway=*` lines -> `water` class. Ditches and drains are left out.
pub const WATERWAY_LINES: &[(&str, WaterClass)] = &[
    ("river", WaterClass::River),
    ("stream", WaterClass::Stream),
    ("canal", WaterClass::Canal),
];

/// `natural=water` areas by their `water=*`: a river's or canal's area is
/// `riverbank`, a reservoir is `reservoir`, everything else (lakes, ponds,
/// oxbows, no `water` tag) is `lake`.
pub const WATER_KIND: &[(&str, WaterClass)] = &[
    ("reservoir", WaterClass::Reservoir),
    ("river", WaterClass::Riverbank),
    ("riverbank", WaterClass::Riverbank),
    ("canal", WaterClass::Riverbank),
    ("stream", WaterClass::Riverbank),
];

/// Land cover areas: (key, value, class). `landuse=farmland` is not drawn
/// (style.rs) and is not in packs.
pub const LAND: &[(&str, &str, LandClass)] = &[
    ("landuse", "forest", LandClass::Forest),
    ("natural", "wood", LandClass::Wood),
    ("leisure", "park", LandClass::Park),
    ("natural", "wetland", LandClass::Wetland),
    ("natural", "glacier", LandClass::Glacier),
    ("natural", "sand", LandClass::Sand),
    ("natural", "beach", LandClass::Sand),
];

/// `place=*` -> `places` class (points; an area gives its centre).
pub const PLACES: &[(&str, PlaceClass)] = &[
    ("city", PlaceClass::City),
    ("town", PlaceClass::Town),
    ("village", PlaceClass::Village),
    ("hamlet", PlaceClass::Hamlet),
];

/// `place=*` values that are searchable settlements but not drawn (the
/// style has no class for them) and never used as `near`.
pub const PLACES_SEARCH_ONLY: &[&str] = &["suburb", "quarter", "neighbourhood"];

/// Points of interest, most urgent category first (ADR 0016 §4): (key,
/// value, category). The value is the `rs_place.class` of the row, and
/// [`poi_category`] maps it back, so every value appears once.
/// `man_made=water_tap` counts only with `drinking_water=yes` (see
/// [`poi`]).
pub const POIS: &[(&str, &str, PoiCategory)] = &[
    ("amenity", "hospital", PoiCategory::Health),
    ("amenity", "clinic", PoiCategory::Health),
    ("amenity", "doctors", PoiCategory::Health),
    ("amenity", "pharmacy", PoiCategory::Health),
    ("healthcare", "hospital", PoiCategory::Health),
    ("healthcare", "clinic", PoiCategory::Health),
    ("healthcare", "doctor", PoiCategory::Health),
    ("healthcare", "pharmacy", PoiCategory::Health),
    ("amenity", "drinking_water", PoiCategory::Water),
    ("amenity", "water_point", PoiCategory::Water),
    ("natural", "spring", PoiCategory::Water),
    ("man_made", "water_tap", PoiCategory::Water),
    ("amenity", "shelter", PoiCategory::Shelter),
    ("tourism", "alpine_hut", PoiCategory::Shelter),
    ("tourism", "wilderness_hut", PoiCategory::Shelter),
    ("tourism", "camp_site", PoiCategory::Shelter),
    ("amenity", "police", PoiCategory::Safety),
    ("amenity", "fire_station", PoiCategory::Safety),
    ("emergency", "defibrillator", PoiCategory::Safety),
    ("shop", "supermarket", PoiCategory::Food),
    ("shop", "bakery", PoiCategory::Food),
    ("amenity", "marketplace", PoiCategory::Food),
    ("amenity", "fuel", PoiCategory::Fuel),
    ("amenity", "toilets", PoiCategory::Toilets),
    ("amenity", "post_office", PoiCategory::Post),
    ("amenity", "townhall", PoiCategory::TownHall),
];

/// Keys whose other values are "other named places" (category `other`),
/// only with a name, in this order of preference. `railway` only for
/// stations and halts.
pub const OTHER_POI_KEYS: &[&str] = &["amenity", "shop", "tourism", "historic", "railway"];
pub const OTHER_RAILWAY: &[&str] = &["station", "halt"];

/// `natural=*` points -> `peaks` class.
pub const PEAKS: &[(&str, PeakClass)] = &[
    ("peak", PeakClass::Peak),
    ("volcano", PeakClass::Volcano),
    ("saddle", PeakClass::Saddle),
];

fn lookup<T: Copy>(table: &[(&str, T)], value: Option<&str>) -> Option<T> {
    let v = value?;
    table.iter().find(|(k, _)| *k == v).map(|(_, c)| *c)
}

/// The category of a POI row's class (an OSM value). Values not in
/// [`POIS`] are "other".
pub fn poi_category(class: &str) -> PoiCategory {
    POIS.iter()
        .find(|(_, v, _)| *v == class)
        .map_or(PoiCategory::Other, |(_, _, c)| *c)
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// One feature to write: its class, and the OSM value it came from (the
/// `rs_place.class` of a POI or settlement row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub class: Class,
    pub osm_class: Cow<'static, str>,
}

impl Item {
    fn of(class: Class) -> Item {
        Item {
            class,
            osm_class: Cow::Borrowed(class.as_str()),
        }
    }
}

pub fn road(t: &(impl Tags + ?Sized), g: Geom) -> Option<RoadClass> {
    (g == Geom::Line).then_some(())?;
    lookup(ROADS, t.tag("highway"))
}

pub fn rail(t: &(impl Tags + ?Sized), g: Geom) -> Option<RailClass> {
    (g == Geom::Line).then_some(())?;
    if t.tag("service")
        .is_some_and(|s| RAIL_SERVICE_LEFT_OUT.contains(&s))
    {
        return None;
    }
    lookup(RAIL, t.tag("railway"))
}

pub fn water(t: &(impl Tags + ?Sized), g: Geom) -> Option<WaterClass> {
    match g {
        Geom::Line => {
            if t.is("natural", "coastline") {
                return Some(WaterClass::Coastline);
            }
            lookup(WATERWAY_LINES, t.tag("waterway"))
        }
        Geom::Area => {
            if t.is("natural", "water") {
                Some(lookup(WATER_KIND, t.tag("water")).unwrap_or(WaterClass::Lake))
            } else if t.is("landuse", "reservoir") {
                Some(WaterClass::Reservoir)
            } else if t.is("waterway", "riverbank") {
                Some(WaterClass::Riverbank)
            } else {
                None
            }
        }
        Geom::Point => None,
    }
}

pub fn land(t: &(impl Tags + ?Sized), g: Geom) -> Option<LandClass> {
    (g == Geom::Area).then_some(())?;
    LAND.iter().find(|(k, v, _)| t.is(k, v)).map(|(_, _, c)| *c)
}

/// Administrative boundaries come from areas (boundary relations and
/// closed ways); their rings are written as lines. Member ways tagged
/// with `boundary` are lines and are left out, so no border is drawn
/// twice from the same relation.
pub fn boundary(t: &(impl Tags + ?Sized), g: Geom) -> Option<AdminLevel> {
    (g == Geom::Area && t.is("boundary", "administrative")).then_some(())?;
    let level = t.tag("admin_level")?.trim();
    if level.len() > 2 {
        return None;
    }
    AdminLevel::from_level(level.parse().ok()?)
}

pub fn place(t: &(impl Tags + ?Sized), g: Geom) -> Option<PlaceClass> {
    (g != Geom::Line).then_some(())?;
    lookup(PLACES, t.tag("place"))
}

/// A searchable settlement that is not drawn (`suburb`...).
pub fn place_search_only(t: &(impl Tags + ?Sized), g: Geom) -> Option<&'static str> {
    (g != Geom::Line).then_some(())?;
    let v = t.tag("place")?;
    PLACES_SEARCH_ONLY.iter().find(|p| **p == v).copied()
}

fn plain_value(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= MAX_CLASS_BYTES
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A point of interest: its category and class (the OSM value). `named`
/// says whether the object has a name: "other" places need one.
pub fn poi(t: &(impl Tags + ?Sized), g: Geom, named: bool) -> Option<(PoiCategory, &str)> {
    (g != Geom::Line).then_some(())?;
    for (k, v, cat) in POIS {
        if t.is(k, v) {
            if *k == "man_made" && !t.is("drinking_water", "yes") {
                continue;
            }
            return Some((*cat, v));
        }
    }
    if !named {
        return None;
    }
    for k in OTHER_POI_KEYS {
        let Some(v) = t.tag(k) else { continue };
        if !plain_value(v) || (*k == "railway" && !OTHER_RAILWAY.contains(&v)) {
            continue;
        }
        // A value of the table under another key (`shop=pharmacy`) takes
        // that value's category, so class and category always agree.
        return Some((poi_category(v), v));
    }
    None
}

pub fn peak(t: &(impl Tags + ?Sized), g: Geom) -> Option<PeakClass> {
    (g == Geom::Point).then_some(())?;
    lookup(PEAKS, t.tag("natural"))
}

/// Every feature an object gives, one per layer at most, in layer order.
pub fn classify(t: &(impl Tags + ?Sized), g: Geom, named: bool) -> Vec<Item> {
    let mut out = Vec::new();
    if let Some(c) = water(t, g) {
        out.push(Item::of(Class::Water(c)));
    }
    if let Some(c) = land(t, g) {
        out.push(Item::of(Class::Land(c)));
    }
    if let Some(c) = road(t, g) {
        out.push(Item::of(Class::Road(c)));
    }
    if let Some(c) = rail(t, g) {
        out.push(Item::of(Class::Rail(c)));
    }
    if let Some(c) = boundary(t, g) {
        out.push(Item::of(Class::Boundary(c)));
    }
    if let Some(c) = place(t, g)
        && named
    {
        out.push(Item::of(Class::Place(c)));
    }
    if let Some((cat, class)) = poi(t, g, named) {
        out.push(Item {
            class: Class::Poi(cat),
            osm_class: Cow::Owned(class.to_string()),
        });
    }
    if let Some(c) = peak(t, g) {
        out.push(Item::of(Class::Peak(c)));
    }
    out
}

/// The smallest tile zoom at which a class must be in the tiles, for a
/// pack whose data reaches `max_abs_lat` degrees of latitude.
///
/// `style::Rule::min_zoom` is right at 45°. Further north the viewer picks
/// a lower tile zoom for the same scale (`cos(lat)` in ADR §3), so a class
/// would appear one zoom step late. This lowers the zoom exactly as much
/// as the pack's northernmost (or southernmost) latitude needs, using the
/// style's own function: at latitude L the zoom for scale m is the 45°
/// zoom for scale m * cos(45°) / cos(L). Never higher than the style's.
/// `None` for a class the style does not draw (farmland).
pub fn min_zoom(class: Class, max_abs_lat: f64) -> Option<u8> {
    let rule = style::rule(class)?;
    let base = rule.min_zoom();
    let lat = if max_abs_lat.is_finite() {
        max_abs_lat.abs().min(85.0)
    } else {
        85.0
    };
    if lat <= 45.0 {
        return Some(base);
    }
    let ratio = std::f64::consts::FRAC_1_SQRT_2 / lat.to_radians().cos();
    Some(base.min(style::min_zoom_for_scale(rule.max_scale * ratio)))
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// Clean an untrusted name: control and invisible formatting characters
/// are removed, white space runs become one space, the result is cut to
/// `max` characters. `None` if no letter or digit is left.
pub fn clean_name(s: &str, max: usize) -> Option<String> {
    let mut out = String::new();
    let mut n = 0;
    let mut space = false;
    for c in s.chars() {
        if n >= max {
            break;
        }
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if crate::text::is_unsafe(c) || matches!(c, '\u{AD}' | '\u{180E}' | '\u{FE00}'..='\u{FE0F}')
        {
            continue;
        }
        if space {
            out.push(' ');
            n += 1;
            space = false;
            if n >= max {
                break;
            }
        }
        out.push(c);
        n += 1;
    }
    let out = out.trim_end().to_string();
    out.chars().any(char::is_alphanumeric).then_some(out)
}

/// Whether every letter of `s` is in the Latin script (ASCII letters,
/// Latin-1 and the Latin Extended blocks, IPA, fullwidth Latin).
pub fn is_latin(s: &str) -> bool {
    s.chars().filter(|c| c.is_alphabetic()).all(|c| {
        c.is_ascii_alphabetic()
            || matches!(c,
                'ª' | 'º'
                | '\u{00C0}'..='\u{024F}'
                | '\u{0250}'..='\u{02AF}'
                | '\u{1D00}'..='\u{1DBF}'
                | '\u{1E00}'..='\u{1EFF}'
                | '\u{2C60}'..='\u{2C7F}'
                | '\u{A720}'..='\u{A7FF}'
                | '\u{AB30}'..='\u{AB6F}'
                | '\u{FB00}'..='\u{FB06}'
                | '\u{FF21}'..='\u{FF3A}'
                | '\u{FF41}'..='\u{FF5A}')
    })
}

/// The name to show (ADR 0016 §3): `name`, unless it is not in the Latin
/// script and `name:latin` or `name:en` (in that order) is.
pub fn pick_name(t: &(impl Tags + ?Sized)) -> Option<String> {
    let name = clean_name(t.tag("name")?, MAX_NAME_CHARS)?;
    if is_latin(&name) {
        return Some(name);
    }
    for k in ["name:latin", "name:en"] {
        if let Some(alt) = t.tag(k).and_then(|v| clean_name(v, MAX_NAME_CHARS))
            && is_latin(&alt)
        {
            return Some(alt);
        }
    }
    Some(name)
}

/// A road's `ref` (`A 1`, `N7;E 25`), cleaned and short.
pub fn pick_ref(t: &(impl Tags + ?Sized)) -> Option<String> {
    clean_name(t.tag("ref")?, MAX_REF_CHARS)
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

/// Lowest and highest elevation accepted, in metres (the Dead Sea shore
/// and above Everest are typing errors).
pub const ELE_RANGE: (f64, f64) = (-500.0, 9000.0);

const THIN_SPACES: [char; 3] = [' ', '\u{A0}', '\u{202F}'];

/// Parse a number as people write it in OSM: spaces as thousands
/// separators, a decimal comma or point, `1,234` as thousands when the
/// comma is followed by exactly three digits and the integer part has one
/// to three digits not starting with 0. `None` for anything else
/// (several values, words, ranges).
fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, s.strip_prefix('+').unwrap_or(s).trim_start()),
    };
    if body.is_empty() || body.len() > 24 {
        return None;
    }
    if !body.chars().next()?.is_ascii_digit() || !body.chars().last()?.is_ascii_digit() {
        return None;
    }
    // Spaces only between digit groups.
    let mut compact = String::new();
    let mut prev_space = false;
    for c in body.chars() {
        if THIN_SPACES.contains(&c) {
            if prev_space {
                return None;
            }
            prev_space = true;
            continue;
        }
        if !(c.is_ascii_digit() || c == '.' || c == ',') {
            return None;
        }
        prev_space = false;
        compact.push(c);
    }
    let commas = compact.matches(',').count();
    let dots = compact.matches('.').count();
    let normal = match (commas, dots) {
        (0, 0) | (0, 1) => compact,
        (1, 0) => {
            let (int, frac) = compact.split_once(',')?;
            if frac.len() == 3 && (1..=3).contains(&int.len()) && !int.starts_with('0') {
                format!("{int}{frac}")
            } else {
                format!("{int}.{frac}")
            }
        }
        (1, 1) => {
            let (c, d) = (compact.find(',')?, compact.find('.')?);
            if c < d {
                compact.replacen(',', "", 1)
            } else {
                compact.replacen('.', "", 1).replacen(',', ".", 1)
            }
        }
        _ => return None,
    };
    if normal.starts_with('.') || normal.ends_with('.') {
        return None;
    }
    let v: f64 = normal.parse().ok()?;
    v.is_finite().then_some(if neg { -v } else { v })
}

/// A peak's elevation in whole metres from its `ele` tag: `1234`,
/// `1 234 m`, `812,5`, `1234.5m`, `3000 ft`, `3000'`. `None` if it does
/// not parse or is outside [`ELE_RANGE`].
pub fn parse_ele(raw: &str) -> Option<i32> {
    if raw.len() > 32 {
        return None;
    }
    let s = raw.trim().to_ascii_lowercase();
    let mut feet = false;
    let mut num = s.as_str();
    for (suffix, is_feet) in [
        ("metres", false),
        ("meters", false),
        ("metre", false),
        ("meter", false),
        ("m", false),
        ("feet", true),
        ("foot", true),
        ("ft", true),
        ("'", true),
    ] {
        if let Some(rest) = num.strip_suffix(suffix) {
            num = rest.trim_end();
            feet = is_feet;
            break;
        }
    }
    let mut v = parse_number(num)?;
    if feet {
        v *= 0.3048;
    }
    if !(ELE_RANGE.0..=ELE_RANGE.1).contains(&v) {
        return None;
    }
    // In range, so the cast is exact after rounding.
    Some(v.round() as i32)
}

/// A settlement's `population`: digits, possibly grouped by spaces,
/// commas, points or apostrophes (`12 345`, `12,345`, `12.345`,
/// `12'345`). `None` for anything else (`ca. 500`, `500-600`, `1.5k`).
pub fn parse_population(raw: &str) -> Option<u64> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 20 {
        return None;
    }
    let first = s.chars().next()?;
    let last = s.chars().last()?;
    if !first.is_ascii_digit() || !last.is_ascii_digit() {
        return None;
    }
    let mut digits = String::new();
    let mut prev_sep = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            prev_sep = false;
        } else if matches!(c, ',' | '.' | '\'' | ' ' | '\u{A0}' | '\u{202F}') && !prev_sep {
            prev_sep = true;
        } else {
            return None;
        }
    }
    if digits.len() > 10 {
        return None;
    }
    digits.parse().ok()
}

/// `rs_place.rank` and the tile `rank` of a settlement: lower is more
/// important. Classes in steps of 100 (city 0, town 100, village 200,
/// hamlet 300, then the search-only classes), and inside a class by
/// population, larger first (in tenths of a decade, so 10x the
/// population is 10 ranks better); without a population, last of its
/// class.
pub fn settlement_rank(class_index: usize, population: Option<u64>) -> i64 {
    let within = match population {
        Some(p) if p > 0 => {
            let tenths = ((p as f64).log10() * 10.0).floor() as i64;
            99 - tenths.clamp(0, 99)
        }
        _ => 99,
    };
    i64::try_from(class_index).unwrap_or(9) * 100 + within
}

/// The settlement class index for [`settlement_rank`]: the four drawn
/// classes, then [`PLACES_SEARCH_ONLY`].
pub fn settlement_class_index(osm_class: &str) -> Option<usize> {
    PLACES
        .iter()
        .map(|(v, _)| *v)
        .chain(PLACES_SEARCH_ONLY.iter().copied())
        .position(|v| v == osm_class)
}

/// `rs_place.rank` of a street: its road class (motorway 0 ... path 9).
pub fn road_rank(c: RoadClass) -> i64 {
    RoadClass::ALL.iter().position(|r| *r == c).unwrap_or(9) as i64
}

/// `rs_place.rank` of a POI: its category, most urgent first.
pub fn poi_rank(c: PoiCategory) -> i64 {
    PoiCategory::ALL.iter().position(|r| *r == c).unwrap_or(9) as i64
}

/// `rs_place.rank` of natural features: water bodies, rivers, peaks,
/// passes.
pub fn natural_rank(class: &str) -> i64 {
    match class {
        "lake" | "reservoir" => 0,
        "river" => 1,
        "peak" | "volcano" => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags<'a>(t: &'a [(&'a str, &'a str)]) -> &'a [(&'a str, &'a str)] {
        t
    }

    fn classes(t: &[(&str, &str)], g: Geom) -> Vec<&'static str> {
        classify(t, g, t.tag("name").is_some())
            .into_iter()
            .map(|i| i.class.as_str())
            .collect()
    }

    #[test]
    fn roads_by_highway_class_and_geometry() {
        let c = |v: &str, g| road(tags(&[("highway", v)]), g);
        assert_eq!(c("motorway_link", Geom::Line), Some(RoadClass::Motorway));
        assert_eq!(c("living_street", Geom::Line), Some(RoadClass::Residential));
        assert_eq!(c("steps", Geom::Line), Some(RoadClass::Path));
        assert_eq!(c("road", Geom::Line), Some(RoadClass::Unclassified));
        assert_eq!(c("construction", Geom::Line), None);
        assert_eq!(c("Primary", Geom::Line), None);
        // A pedestrian square (area) is not a line.
        assert_eq!(c("pedestrian", Geom::Area), None);
        assert_eq!(c("bus_stop", Geom::Point), None);
        // Every class of the style appears, and the table has no duplicate.
        for rc in RoadClass::ALL {
            assert!(ROADS.iter().any(|(_, c)| c == rc), "{rc:?}");
        }
        let mut keys: Vec<_> = ROADS.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ROADS.len());
    }

    #[test]
    fn water_land_rail_and_boundaries() {
        assert_eq!(classes(&[("natural", "water")], Geom::Area), vec!["lake"]);
        assert_eq!(
            classes(&[("natural", "water"), ("water", "reservoir")], Geom::Area),
            vec!["reservoir"]
        );
        assert_eq!(
            classes(&[("natural", "water"), ("water", "river")], Geom::Area),
            vec!["riverbank"]
        );
        assert_eq!(classes(&[("waterway", "river")], Geom::Line), vec!["river"]);
        assert_eq!(
            classes(&[("waterway", "ditch")], Geom::Line),
            Vec::<&str>::new()
        );
        assert_eq!(
            classes(&[("natural", "coastline")], Geom::Line),
            vec!["coastline"]
        );
        // A lake drawn as an unclosed way is broken data: left out.
        assert!(classes(&[("natural", "water")], Geom::Line).is_empty());
        assert_eq!(
            classes(&[("landuse", "forest")], Geom::Area),
            vec!["forest"]
        );
        assert_eq!(classes(&[("natural", "beach")], Geom::Area), vec!["sand"]);
        assert!(classes(&[("landuse", "farmland")], Geom::Area).is_empty());
        assert_eq!(
            classes(&[("railway", "tram")], Geom::Line),
            vec!["light_rail"]
        );
        assert!(classes(&[("railway", "rail"), ("service", "yard")], Geom::Line).is_empty());
        assert!(classes(&[("railway", "subway")], Geom::Line).is_empty());
        let b = |level: &str, g| {
            boundary(
                tags(&[("boundary", "administrative"), ("admin_level", level)]),
                g,
            )
        };
        assert_eq!(b("8", Geom::Area), Some(AdminLevel::Municipality));
        assert_eq!(b(" 2 ", Geom::Area), Some(AdminLevel::Country));
        assert_eq!(b("7", Geom::Area), None);
        assert_eq!(b("08x", Geom::Area), None);
        assert_eq!(b("8", Geom::Line), None);
        assert_eq!(boundary(tags(&[("admin_level", "8")]), Geom::Area), None);
    }

    #[test]
    fn places_pois_and_peaks() {
        assert_eq!(
            classes(&[("place", "town"), ("name", "Testville")], Geom::Point),
            vec!["town"]
        );
        // Unnamed places are not drawn (a marker without a name means
        // nothing); places from areas count (their centre is used).
        assert!(classes(&[("place", "town")], Geom::Point).is_empty());
        assert_eq!(
            classes(&[("place", "hamlet"), ("name", "X")], Geom::Area),
            vec!["hamlet"]
        );
        assert_eq!(
            place_search_only(tags(&[("place", "suburb")]), Geom::Point),
            Some("suburb")
        );
        let p =
            |t: &[(&str, &str)], named| poi(t, Geom::Point, named).map(|(c, v)| (c, v.to_string()));
        assert_eq!(
            p(&[("amenity", "pharmacy")], false),
            Some((PoiCategory::Health, "pharmacy".into()))
        );
        assert_eq!(
            p(&[("amenity", "drinking_water")], false),
            Some((PoiCategory::Water, "drinking_water".into()))
        );
        assert_eq!(p(&[("man_made", "water_tap")], false), None);
        assert_eq!(
            p(
                &[("man_made", "water_tap"), ("drinking_water", "yes")],
                false
            ),
            Some((PoiCategory::Water, "water_tap".into()))
        );
        // Other places need a name, and a plain value.
        assert_eq!(p(&[("amenity", "cafe")], false), None);
        assert_eq!(
            p(&[("amenity", "cafe")], true),
            Some((PoiCategory::Other, "cafe".into()))
        );
        assert_eq!(p(&[("amenity", "Cafe; bar")], true), None);
        assert_eq!(p(&[("railway", "level_crossing")], true), None);
        assert_eq!(
            p(&[("railway", "station")], true),
            Some((PoiCategory::Other, "station".into()))
        );
        // The most urgent category wins; a known value under another key
        // keeps its category.
        assert_eq!(
            p(&[("shop", "bakery"), ("amenity", "pharmacy")], true),
            Some((PoiCategory::Health, "pharmacy".into()))
        );
        assert_eq!(
            p(&[("shop", "pharmacy")], true),
            Some((PoiCategory::Health, "pharmacy".into()))
        );
        assert_eq!(
            p(&[("amenity", "pharmacy")], false).map(|x| x.0),
            Some(PoiCategory::Health)
        );
        assert_eq!(
            poi(tags(&[("amenity", "pharmacy")]), Geom::Line, true),
            None
        );
        assert_eq!(
            classes(&[("natural", "peak"), ("ele", "500")], Geom::Point),
            vec!["peak"]
        );
        assert!(classes(&[("natural", "peak")], Geom::Area).is_empty());
        // A POI area and its land cover are both kept.
        assert_eq!(
            classes(&[("leisure", "park"), ("amenity", "toilets")], Geom::Area),
            vec!["park", "toilets"]
        );
    }

    #[test]
    fn poi_values_are_unique_and_map_back_to_their_category() {
        let mut values: Vec<&str> = POIS.iter().map(|(_, v, _)| *v).collect();
        values.sort_unstable();
        let n = values.len();
        values.dedup();
        // `hospital`, `clinic` and `pharmacy` appear under two keys with
        // the same category.
        for (_, v, c) in POIS {
            assert_eq!(poi_category(v), *c, "{v}");
        }
        assert!(values.len() < n);
        assert_eq!(poi_category("cafe"), PoiCategory::Other);
        for cat in PoiCategory::ALL {
            if *cat != PoiCategory::Other {
                assert!(POIS.iter().any(|(_, _, c)| c == cat), "{cat:?}");
            }
        }
    }

    #[test]
    fn min_zoom_follows_the_style_with_a_latitude_margin() {
        let road = Class::Road(RoadClass::Residential);
        let rule = style::rule(road).map(style::Rule::min_zoom);
        assert_eq!(min_zoom(road, 30.0), rule);
        assert_eq!(min_zoom(road, 45.0), rule);
        // Never higher than the style's, and lower far north.
        for lat in [46.0, 50.0, 60.0, 70.0, 89.0, f64::NAN] {
            for r in style::RULES {
                let z = min_zoom(r.class, lat).expect("drawn class");
                assert!(z <= r.min_zoom());
            }
        }
        // At 70 degrees, scales are cos(70)/cos(45) = 0.48 times smaller:
        // one zoom step lower for everything that is not already at 0.
        assert_eq!(min_zoom(road, 70.0), rule.map(|z| z - 1));
        assert_eq!(
            min_zoom(Class::Boundary(AdminLevel::Country), 70.0),
            Some(0)
        );
        assert_eq!(min_zoom(Class::Land(LandClass::Farmland), 50.0), None);
        // The viewer's zoom at the class's scale and the pack's latitude
        // is never below the minimum zoom (the class is in the tiles).
        for lat in [45.0_f64, 49.4, 50.2, 55.0, 64.0] {
            for r in style::RULES.iter().filter(|r| r.max_scale.is_finite()) {
                let z = min_zoom(r.class, lat).expect("drawn class");
                let viewer = (156_543.03 * lat.to_radians().cos() / r.max_scale)
                    .log2()
                    .floor();
                let viewer = viewer.clamp(0.0, 14.0) as u8;
                assert!(viewer >= z, "{:?} at {lat}: viewer {viewer} < {z}", r.class);
            }
        }
    }

    #[test]
    fn names_prefer_latin_script() {
        assert_eq!(
            pick_name(tags(&[("name", "Rue de l'Essai")])).as_deref(),
            Some("Rue de l'Essai")
        );
        assert_eq!(
            pick_name(tags(&[("name", "Москва"), ("name:en", "Moscow")])).as_deref(),
            Some("Moscow")
        );
        assert_eq!(
            pick_name(tags(&[
                ("name", "Αθήνα"),
                ("name:latin", "Athína"),
                ("name:en", "Athens")
            ]))
            .as_deref(),
            Some("Athína")
        );
        // No Latin alternative: the name as it is (the viewer folds it).
        assert_eq!(
            pick_name(tags(&[("name", "東京")])).as_deref(),
            Some("東京")
        );
        // A non-Latin alternative does not replace anything.
        assert_eq!(
            pick_name(tags(&[("name", "Αθήνα"), ("name:en", "Αθήνα")])).as_deref(),
            Some("Αθήνα")
        );
        assert_eq!(pick_name(tags(&[("name:en", "Only English")])), None);
        assert_eq!(
            pick_name(tags(&[("name", "Île-de-France")])).as_deref(),
            Some("Île-de-France")
        );
    }

    #[test]
    fn names_are_cleaned_and_bounded() {
        assert_eq!(
            clean_name("  Evil\x1b[2J\n name\u{202E} ", 50).as_deref(),
            Some("Evil[2J name")
        );
        assert_eq!(clean_name("a\u{200B}b\u{AD}c", 50).as_deref(), Some("abc"));
        assert_eq!(clean_name("   ", 50), None);
        assert_eq!(clean_name("--- ...", 50), None);
        let long = "x".repeat(10_000);
        assert_eq!(
            clean_name(&long, MAX_NAME_CHARS).map(|s| s.chars().count()),
            Some(MAX_NAME_CHARS)
        );
        assert_eq!(clean_name("ab cd", 3).as_deref(), Some("ab"));
        let huge = "y".repeat(MAX_TAG_BYTES + 1);
        assert_eq!(pick_name(tags(&[("name", huge.as_str())])), None);
        assert_eq!(
            pick_ref(tags(&[("ref", "N 7;E 25")])).as_deref(),
            Some("N 7;E 25")
        );
    }

    #[test]
    fn elevations_are_parsed_defensively() {
        for (raw, want) in [
            ("1234", Some(1234)),
            ("1 234 m", Some(1234)),
            ("1\u{A0}234m", Some(1234)),
            ("812,5", Some(813)),
            ("812.4", Some(812)),
            ("8,848", Some(8848)),
            ("0,123", Some(0)),
            ("1,234.5", Some(1235)),
            ("1.234,5", Some(1235)),
            ("3000 ft", Some(914)),
            ("3000'", Some(914)),
            ("1000 feet", Some(305)),
            ("-28", Some(-28)),
            ("  560 metres ", Some(560)),
            ("1234M", Some(1234)),
            ("", None),
            ("m", None),
            ("ca. 1200", None),
            ("~1200", None),
            ("1200-1300", None),
            ("1200;1250", None),
            ("1.2.3", None),
            ("1,2,3", None),
            ("12  34", None),
            (".5", None),
            ("5.", None),
            ("NaN", None),
            ("inf", None),
            ("1e3", None),
            ("99999", None),
            ("-1000", None),
            ("1 234 567 890 123 456 789 0", None),
        ] {
            assert_eq!(parse_ele(raw), want, "{raw:?}");
        }
        assert_eq!(parse_ele(&"1".repeat(100)), None);
    }

    #[test]
    fn populations_and_ranks() {
        for (raw, want) in [
            ("12345", Some(12345)),
            ("12 345", Some(12345)),
            ("12,345", Some(12345)),
            ("12.345", Some(12345)),
            ("12'345", Some(12345)),
            ("0", Some(0)),
            ("ca. 500", None),
            ("500-600", None),
            ("1.5k", None),
            ("12,,345", None),
            ("", None),
            ("-5", None),
            ("12345678901", None),
        ] {
            assert_eq!(parse_population(raw), want, "{raw:?}");
        }
        let town = settlement_class_index("town").expect("town");
        assert_eq!(settlement_rank(town, None), 199);
        assert!(settlement_rank(town, Some(100_000)) < settlement_rank(town, Some(10_000)));
        assert!(settlement_rank(0, None) < settlement_rank(town, Some(10_000_000)));
        assert_eq!(settlement_rank(town, Some(10_000_000_000)), 100);
        assert_eq!(settlement_class_index("suburb"), Some(4));
        assert_eq!(settlement_class_index("farm"), None);
        assert_eq!(road_rank(RoadClass::Motorway), 0);
        assert_eq!(road_rank(RoadClass::Path), 9);
        assert_eq!(poi_rank(PoiCategory::Health), 0);
    }
}
