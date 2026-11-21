//! The `resilience-1` map style as data (ADR 0016 §1, §3).
//!
//! [`RULES`] says, for each layer and class of the tile schema, the
//! coarsest scale (metres per pixel at the view's centre) at which it is
//! drawn, its colour, how it is drawn and its drawing rank. The viewer
//! looks features up with [`lookup`], draws them in the order of
//! [`Rule::sort_key`] (areas, then lines by rank, then labels), and shows
//! [`legend`], which is generated from the same table so the two cannot
//! disagree.
//!
//! **The class names are a contract.** The pack builder
//! (`resilience-mapmaker`) writes exactly the strings of [`Layer::as_str`]
//! and the class enums' `as_str` into the tiles, and takes each class's
//! minimum tile zoom from [`Rule::min_zoom`]; the viewer parses them back
//! with the same functions. Both sides must use these definitions and
//! never spell a name out themselves. A class the table does not know is
//! not drawn.

use super::canvas::{Canvas, Cell, CellMask, Colour, LinePattern};

/// "Always drawn": the coarsest scale is unbounded.
pub const ALWAYS: f64 = f64::INFINITY;

/// A scale counts as "at most" a threshold within this relative margin, so
/// a scale computed with rounding (499.9999 or 500.0001 m) behaves like
/// the zoom step it stands for.
const SCALE_TOLERANCE: f64 = 1e-6;

/// Colour of the sea: the viewer paints the canvas with it before drawing
/// the overview's land polygons ([`LandClass::Landmass`]) over it.
pub const SEA: Colour = Colour::Blue;

/// Outside every installed detailed pack, at this scale or finer, the map
/// is shaded `░` in [`NO_DETAIL_SHADE`] (ADR 0016 §3).
pub const NO_DETAIL_MAX_SCALE: f64 = 500.0;
pub const NO_DETAIL_SHADE: Colour = Colour::White;

/// At scales coarser than [`NO_DETAIL_MAX_SCALE`], the edge of each
/// detailed pack's coverage is drawn instead, in this colour and pattern.
pub const COVERAGE_EDGE: (Colour, LinePattern) = (Colour::White, LinePattern::Dotted);

/// The crosshair at the centre: this character in reverse video.
pub const CROSSHAIR: char = '+';

/// Width in cells of a legend sample.
pub const LEGEND_SAMPLE_WIDTH: u16 = 6;

macro_rules! names {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name { $($(#[$vm])* $v),+ }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$v),+];

            /// The name written in the tiles.
            pub const fn as_str(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }

            /// The value for a name from a tile (exact match).
            pub fn parse(s: &str) -> Option<$name> {
                match s { $($s => Some($name::$v),)+ _ => None }
            }
        }
    };
}

names! {
    /// Tile layers of `resilience-1`.
    Layer {
        Water = "water",
        Land = "land",
        Roads = "roads",
        Rail = "rail",
        Boundaries = "boundaries",
        Places = "places",
        Pois = "pois",
        Peaks = "peaks",
    }
}

names! {
    /// `class` of the `water` layer: polygons (sea, lake, reservoir,
    /// riverbank) and lines (the others). `sea` comes from OpenStreetMap's
    /// water polygons, cut to a regional pack (the extract itself has only
    /// coastline lines).
    WaterClass {
        Sea = "sea",
        Lake = "lake",
        Reservoir = "reservoir",
        Riverbank = "riverbank",
        Coastline = "coastline",
        River = "river",
        Stream = "stream",
        Canal = "canal",
    }
}

names! {
    /// `class` of the `land` layer (polygons). `landmass` is the overview's
    /// land (Natural Earth), drawn over the sea colour.
    LandClass {
        Landmass = "landmass",
        Forest = "forest",
        Wood = "wood",
        Park = "park",
        Wetland = "wetland",
        Glacier = "glacier",
        Sand = "sand",
        Farmland = "farmland",
    }
}

names! {
    /// `class` of the `roads` layer (lines). The builder maps OSM's
    /// `living_street` and `pedestrian` to `residential`, links to their
    /// road's class, and `footway`, `cycleway`, `bridleway`, `steps` to
    /// `path`.
    RoadClass {
        Motorway = "motorway",
        Trunk = "trunk",
        Primary = "primary",
        Secondary = "secondary",
        Tertiary = "tertiary",
        Unclassified = "unclassified",
        Residential = "residential",
        Service = "service",
        Track = "track",
        Path = "path",
    }
}

names! {
    /// `class` of the `rail` layer (lines): `light_rail` also covers trams
    /// and surface metro lines.
    RailClass {
        Rail = "rail",
        LightRail = "light_rail",
    }
}

names! {
    /// `admin_level` of the `boundaries` layer (lines). In the tiles it is
    /// the number; [`AdminLevel::as_str`] is its decimal form.
    AdminLevel {
        Country = "2",
        Region = "4",
        Department = "6",
        Municipality = "8",
    }
}

names! {
    /// `class` of the `places` layer (points).
    PlaceClass {
        City = "city",
        Town = "town",
        Village = "village",
        Hamlet = "hamlet",
    }
}

names! {
    /// `category` of the `pois` layer (points), most urgent first (ADR
    /// 0016 §4). The builder maps OSM tags to these:
    /// health = hospital, clinic, doctors, pharmacy; water = drinking
    /// water, water point, spring; shelter = shelter, alpine hut, camp
    /// site; safety = police, fire station, defibrillator; food =
    /// supermarket, bakery, marketplace; then fuel, toilets, post,
    /// town hall, and other named places.
    PoiCategory {
        Health = "health",
        Water = "water",
        Shelter = "shelter",
        Safety = "safety",
        Food = "food",
        Fuel = "fuel",
        Toilets = "toilets",
        Post = "post",
        TownHall = "town_hall",
        Other = "other",
    }
}

names! {
    /// `class` of the `peaks` layer (points).
    PeakClass {
        Peak = "peak",
        Volcano = "volcano",
        Saddle = "saddle",
        /// A major summit of the world overview (Natural Earth's elevation
        /// points), drawn from overview scales with its height.
        Summit = "summit",
        /// A place below its surroundings (the Dead Sea shore), overview.
        Depression = "depression",
    }
}

impl AdminLevel {
    /// The level for the `admin_level` number of a tile feature.
    pub fn from_level(level: i64) -> Option<AdminLevel> {
        match level {
            2 => Some(AdminLevel::Country),
            4 => Some(AdminLevel::Region),
            6 => Some(AdminLevel::Department),
            8 => Some(AdminLevel::Municipality),
            _ => None,
        }
    }

    pub const fn level(self) -> u8 {
        match self {
            AdminLevel::Country => 2,
            AdminLevel::Region => 4,
            AdminLevel::Department => 6,
            AdminLevel::Municipality => 8,
        }
    }
}

impl PoiCategory {
    /// The letter drawn at the point of interest (also in the legend).
    pub const fn letter(self) -> char {
        match self {
            PoiCategory::Health => 'H',
            PoiCategory::Water => 'W',
            PoiCategory::Shelter => 'S',
            PoiCategory::Safety => 'E',
            PoiCategory::Food => 'F',
            PoiCategory::Fuel => 'G',
            PoiCategory::Toilets => 'T',
            PoiCategory::Post => 'P',
            PoiCategory::TownHall => 'A',
            PoiCategory::Other => 'o',
        }
    }
}

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

/// The category of a POI row's class (an OSM value). Values not in
/// [`POIS`] are "other".
pub fn poi_category(class: &str) -> PoiCategory {
    POIS.iter()
        .find(|(_, v, _)| *v == class)
        .map_or(PoiCategory::Other, |(_, _, c)| *c)
}

/// A class of one layer of `resilience-1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    Water(WaterClass),
    Land(LandClass),
    Road(RoadClass),
    Rail(RailClass),
    Boundary(AdminLevel),
    Place(PlaceClass),
    Poi(PoiCategory),
    Peak(PeakClass),
}

impl Class {
    pub const fn layer(self) -> Layer {
        match self {
            Class::Water(_) => Layer::Water,
            Class::Land(_) => Layer::Land,
            Class::Road(_) => Layer::Roads,
            Class::Rail(_) => Layer::Rail,
            Class::Boundary(_) => Layer::Boundaries,
            Class::Place(_) => Layer::Places,
            Class::Poi(_) => Layer::Pois,
            Class::Peak(_) => Layer::Peaks,
        }
    }

    /// The value written in the tiles: `class`, or `category` for
    /// `pois`, or the decimal `admin_level` for `boundaries`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Class::Water(c) => c.as_str(),
            Class::Land(c) => c.as_str(),
            Class::Road(c) => c.as_str(),
            Class::Rail(c) => c.as_str(),
            Class::Boundary(c) => c.as_str(),
            Class::Place(c) => c.as_str(),
            Class::Poi(c) => c.as_str(),
            Class::Peak(c) => c.as_str(),
        }
    }

    /// The tile attribute that holds the class in this layer.
    pub const fn attribute(layer: Layer) -> &'static str {
        match layer {
            Layer::Boundaries => "admin_level",
            Layer::Pois => "category",
            _ => "class",
        }
    }

    /// The class for a layer and the value of its class attribute.
    pub fn parse(layer: Layer, value: &str) -> Option<Class> {
        match layer {
            Layer::Water => WaterClass::parse(value).map(Class::Water),
            Layer::Land => LandClass::parse(value).map(Class::Land),
            Layer::Roads => RoadClass::parse(value).map(Class::Road),
            Layer::Rail => RailClass::parse(value).map(Class::Rail),
            Layer::Boundaries => AdminLevel::parse(value).map(Class::Boundary),
            Layer::Places => PlaceClass::parse(value).map(Class::Place),
            Layer::Pois => PoiCategory::parse(value).map(Class::Poi),
            Layer::Peaks => PeakClass::parse(value).map(Class::Peak),
        }
    }

    /// Every class of every layer.
    pub fn all() -> impl Iterator<Item = Class> {
        let w = WaterClass::ALL.iter().map(|c| Class::Water(*c));
        let l = LandClass::ALL.iter().map(|c| Class::Land(*c));
        let r = RoadClass::ALL.iter().map(|c| Class::Road(*c));
        let ra = RailClass::ALL.iter().map(|c| Class::Rail(*c));
        let b = AdminLevel::ALL.iter().map(|c| Class::Boundary(*c));
        let p = PlaceClass::ALL.iter().map(|c| Class::Place(*c));
        let po = PoiCategory::ALL.iter().map(|c| Class::Poi(*c));
        let pe = PeakClass::ALL.iter().map(|c| Class::Peak(*c));
        w.chain(l)
            .chain(r)
            .chain(ra)
            .chain(b)
            .chain(p)
            .chain(po)
            .chain(pe)
    }

    /// Plain-English words for the legend (ASCII).
    pub const fn description(self) -> &'static str {
        match self {
            Class::Water(c) => match c {
                WaterClass::Sea => "sea",
                WaterClass::Lake => "lake",
                WaterClass::Reservoir => "reservoir",
                WaterClass::Riverbank => "wide river",
                WaterClass::Coastline => "coastline",
                WaterClass::River => "river",
                WaterClass::Stream => "stream",
                WaterClass::Canal => "canal",
            },
            Class::Land(c) => match c {
                LandClass::Landmass => "land",
                LandClass::Forest => "forest",
                LandClass::Wood => "wood",
                LandClass::Park => "park",
                LandClass::Wetland => "wetland",
                LandClass::Glacier => "glacier",
                LandClass::Sand => "sand, beach",
                LandClass::Farmland => "farmland",
            },
            Class::Road(c) => match c {
                RoadClass::Motorway => "motorway",
                RoadClass::Trunk => "trunk road",
                RoadClass::Primary => "primary road",
                RoadClass::Secondary => "secondary road",
                RoadClass::Tertiary => "minor road",
                RoadClass::Unclassified => "unclassified road",
                RoadClass::Residential => "residential street",
                RoadClass::Service => "service road",
                RoadClass::Track => "track",
                RoadClass::Path => "path",
            },
            Class::Rail(c) => match c {
                RailClass::Rail => "railway",
                RailClass::LightRail => "tram, light rail",
            },
            Class::Boundary(c) => match c {
                AdminLevel::Country => "country border",
                AdminLevel::Region => "region border",
                AdminLevel::Department => "department or county border",
                AdminLevel::Municipality => "municipality border",
            },
            Class::Place(c) => match c {
                PlaceClass::City => "city",
                PlaceClass::Town => "town",
                PlaceClass::Village => "village",
                PlaceClass::Hamlet => "hamlet",
            },
            Class::Poi(c) => match c {
                PoiCategory::Health => "health: hospital, doctor, pharmacy",
                PoiCategory::Water => "drinking water, spring",
                PoiCategory::Shelter => "shelter, hut, camp site",
                PoiCategory::Safety => "police, fire station, defibrillator",
                PoiCategory::Food => "food shop, market",
                PoiCategory::Fuel => "fuel",
                PoiCategory::Toilets => "toilets",
                PoiCategory::Post => "post office",
                PoiCategory::TownHall => "town hall",
                PoiCategory::Other => "other named place",
            },
            Class::Peak(c) => match c {
                PeakClass::Peak => "peak",
                PeakClass::Volcano => "volcano",
                PeakClass::Saddle => "mountain pass",
                PeakClass::Summit => "major summit (height in metres)",
                PeakClass::Depression => "depression (height in metres)",
            },
        }
    }
}

/// The three drawing passes, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pass {
    Areas,
    Lines,
    Labels,
}

impl Pass {
    pub const ALL: [Pass; 3] = [Pass::Areas, Pass::Lines, Pass::Labels];
}

/// How a label class is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelRule {
    /// Character drawn at the point (`*` places, `^` peaks, the category
    /// letter for points of interest); `None` for street names.
    pub marker: Option<char>,
    /// Higher is placed first (city > town > peak > village > POI >
    /// street name).
    pub priority: u16,
    /// Coarsest scale at which the name is written next to the marker.
    pub name_max_scale: f64,
    /// Coarsest scale at which the detail (a peak's elevation) is added
    /// to the name; `None` when the class has no detail.
    pub detail_max_scale: Option<f64>,
    /// Keep the marker alone when there is no room for the name (points
    /// of interest and peaks); otherwise the whole label is dropped.
    pub keep_marker: bool,
}

impl LabelRule {
    pub fn name_visible(&self, scale: f64) -> bool {
        at_most(scale, self.name_max_scale)
    }

    pub fn detail_visible(&self, scale: f64) -> bool {
        self.detail_max_scale.is_some_and(|m| at_most(scale, m))
    }
}

/// How a class is drawn. A fill applies to polygon geometry, a line to
/// line geometry, a label to points; the viewer skips a feature whose
/// geometry does not match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Draw {
    Fill,
    Line(LinePattern),
    Label(LabelRule),
}

/// One row of the style table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    pub class: Class,
    /// Drawn while the scale (m per pixel) is at most this; [`ALWAYS`]
    /// for always.
    pub max_scale: f64,
    pub colour: Colour,
    pub draw: Draw,
    /// Order within the pass: higher ranks are drawn later (on top).
    pub rank: u16,
}

impl Rule {
    pub fn visible(&self, scale: f64) -> bool {
        at_most(scale, self.max_scale)
    }

    pub const fn pass(&self) -> Pass {
        match self.draw {
            Draw::Fill => Pass::Areas,
            Draw::Line(_) => Pass::Lines,
            Draw::Label(_) => Pass::Labels,
        }
    }

    /// Sort features by this (stable sort) and draw them in that order.
    pub const fn sort_key(&self) -> (Pass, u16) {
        (self.pass(), self.rank)
    }

    /// Smallest tile zoom whose tiles must contain this class, for the
    /// pack builder (tippecanoe `minzoom`). See [`min_zoom_for_scale`].
    pub fn min_zoom(&self) -> u8 {
        min_zoom_for_scale(self.max_scale)
    }

    pub fn label(&self) -> Option<&LabelRule> {
        match &self.draw {
            Draw::Label(l) => Some(l),
            _ => None,
        }
    }
}

/// Highest tile zoom of `resilience-1` packs.
pub const MAX_TILE_ZOOM: u8 = 14;

/// The tile zoom the viewer uses at `scale` m per pixel at 45° latitude
/// (ADR 0016 §3: `z = floor(log2(156543.03 * cos(lat) / m))`, clamped to
/// 0..=14). A class drawn up to `max_scale` must be in the tiles from this
/// zoom on. At higher latitudes the viewer uses a slightly lower zoom for
/// the same scale, so a class can appear one zoom step late there; at
/// lower latitudes it is always present.
pub fn min_zoom_for_scale(max_scale: f64) -> u8 {
    if !max_scale.is_finite() || max_scale <= 0.0 {
        return 0;
    }
    let z = (156_543.03 * std::f64::consts::FRAC_1_SQRT_2 / max_scale)
        .log2()
        .floor();
    if z <= 0.0 {
        0
    } else if z >= f64::from(MAX_TILE_ZOOM) {
        MAX_TILE_ZOOM
    } else {
        z as u8
    }
}

fn at_most(scale: f64, max: f64) -> bool {
    scale.is_finite() && scale > 0.0 && scale <= max * (1.0 + SCALE_TOLERANCE)
}

const fn fill(class: Class, max_scale: f64, colour: Colour, rank: u16) -> Rule {
    Rule {
        class,
        max_scale,
        colour,
        draw: Draw::Fill,
        rank,
    }
}

const fn line(class: Class, max_scale: f64, colour: Colour, p: LinePattern, rank: u16) -> Rule {
    Rule {
        class,
        max_scale,
        colour,
        draw: Draw::Line(p),
        rank,
    }
}

const fn place(class: PlaceClass, max_scale: f64, priority: u16) -> Rule {
    Rule {
        class: Class::Place(class),
        max_scale,
        colour: Colour::White,
        draw: Draw::Label(LabelRule {
            marker: Some('*'),
            priority,
            name_max_scale: max_scale,
            detail_max_scale: None,
            keep_marker: false,
        }),
        rank: priority,
    }
}

const fn peak(class: PeakClass) -> Rule {
    Rule {
        class: Class::Peak(class),
        max_scale: 200.0,
        colour: Colour::White,
        draw: Draw::Label(LabelRule {
            marker: Some('^'),
            priority: PEAK_PRIORITY,
            name_max_scale: 200.0,
            detail_max_scale: Some(100.0),
            keep_marker: true,
        }),
        rank: PEAK_PRIORITY,
    }
}

/// A spot height of the world overview: visible, with its name and height,
/// from overview scales (ADR 0016 §5); the overview's own minimum zooms
/// keep only the major ones at the coarsest scales.
const fn spot(class: PeakClass, marker: char) -> Rule {
    Rule {
        class: Class::Peak(class),
        max_scale: 10_000.0,
        colour: Colour::White,
        draw: Draw::Label(LabelRule {
            marker: Some(marker),
            priority: PEAK_PRIORITY,
            name_max_scale: 10_000.0,
            detail_max_scale: Some(10_000.0),
            keep_marker: true,
        }),
        rank: PEAK_PRIORITY,
    }
}

const fn poi(cat: PoiCategory, priority: u16) -> Rule {
    Rule {
        class: Class::Poi(cat),
        max_scale: 20.0,
        colour: Colour::Cyan,
        draw: Draw::Label(LabelRule {
            marker: Some(cat.letter()),
            priority,
            name_max_scale: 10.0,
            detail_max_scale: None,
            keep_marker: true,
        }),
        rank: priority,
    }
}

const PEAK_PRIORITY: u16 = 80;

/// Street names (any drawn road with a `name`), at 5 and 10 m per pixel,
/// along the longest nearly horizontal stretch (`label::street_anchor`).
pub const STREET_NAMES: LabelRule = LabelRule {
    marker: None,
    priority: 40,
    name_max_scale: 10.0,
    detail_max_scale: None,
    keep_marker: false,
};
pub const STREET_NAME_COLOUR: Colour = Colour::White;

use Colour::{Black, Blue, Cyan, Green, Magenta, Red, White, Yellow};
use LinePattern::{Dashed, Dotted, Solid};

/// The style table (ADR 0016 §3), one row per class. `farmland` has no
/// row: it is not drawn (it would cover the countryside) and the builder
/// leaves it out.
pub static RULES: &[Rule] = &[
    // Areas.
    fill(Class::Land(LandClass::Landmass), ALWAYS, Black, 0),
    // Under every other area: sand and wetland on the shore are drawn over
    // it. The legend's "Sea" line (the background colour) stands for it.
    fill(Class::Water(WaterClass::Sea), ALWAYS, SEA, 1),
    fill(Class::Land(LandClass::Sand), 50.0, Yellow, 2),
    fill(Class::Land(LandClass::Wetland), 50.0, Green, 3),
    fill(Class::Land(LandClass::Park), 50.0, Green, 4),
    fill(Class::Land(LandClass::Forest), 500.0, Green, 5),
    fill(Class::Land(LandClass::Wood), 500.0, Green, 5),
    fill(Class::Land(LandClass::Glacier), 500.0, White, 6),
    fill(Class::Water(WaterClass::Lake), ALWAYS, Blue, 10),
    fill(Class::Water(WaterClass::Reservoir), ALWAYS, Blue, 10),
    fill(Class::Water(WaterClass::Riverbank), 500.0, Blue, 10),
    // Lines, lowest rank first.
    line(Class::Water(WaterClass::Coastline), 500.0, Blue, Solid, 10),
    line(Class::Water(WaterClass::River), 500.0, Blue, Solid, 11),
    line(Class::Water(WaterClass::Stream), 50.0, Blue, Solid, 12),
    line(Class::Water(WaterClass::Canal), 50.0, Blue, Solid, 12),
    line(Class::Road(RoadClass::Path), 10.0, White, Dotted, 20),
    line(Class::Road(RoadClass::Track), 10.0, White, Dotted, 21),
    line(Class::Road(RoadClass::Service), 10.0, White, Dotted, 21),
    line(Class::Road(RoadClass::Unclassified), 20.0, White, Solid, 22),
    line(Class::Road(RoadClass::Residential), 20.0, White, Solid, 22),
    line(Class::Road(RoadClass::Tertiary), 100.0, White, Solid, 23),
    line(Class::Rail(RailClass::LightRail), 50.0, Cyan, Dashed, 30),
    line(Class::Rail(RailClass::Rail), 200.0, Cyan, Dashed, 31),
    line(Class::Road(RoadClass::Secondary), 200.0, Yellow, Solid, 40),
    line(Class::Road(RoadClass::Primary), 500.0, Yellow, Solid, 41),
    line(Class::Road(RoadClass::Trunk), 2000.0, Red, Solid, 42),
    line(Class::Road(RoadClass::Motorway), 2000.0, Red, Solid, 43),
    line(
        Class::Boundary(AdminLevel::Municipality),
        100.0,
        Magenta,
        Dashed,
        50,
    ),
    line(
        Class::Boundary(AdminLevel::Department),
        1000.0,
        Magenta,
        Dashed,
        51,
    ),
    line(
        Class::Boundary(AdminLevel::Region),
        5000.0,
        Magenta,
        Dashed,
        52,
    ),
    line(
        Class::Boundary(AdminLevel::Country),
        ALWAYS,
        Magenta,
        Dashed,
        53,
    ),
    // Labels, by priority.
    place(PlaceClass::City, 20_000.0, 100),
    place(PlaceClass::Town, 2_000.0, 90),
    peak(PeakClass::Peak),
    peak(PeakClass::Volcano),
    peak(PeakClass::Saddle),
    spot(PeakClass::Summit, '^'),
    spot(PeakClass::Depression, 'v'),
    place(PlaceClass::Village, 200.0, 70),
    place(PlaceClass::Hamlet, 50.0, 60),
    poi(PoiCategory::Health, 59),
    poi(PoiCategory::Water, 58),
    poi(PoiCategory::Shelter, 57),
    poi(PoiCategory::Safety, 56),
    poi(PoiCategory::Food, 55),
    poi(PoiCategory::Fuel, 54),
    poi(PoiCategory::Toilets, 53),
    poi(PoiCategory::Post, 52),
    poi(PoiCategory::TownHall, 51),
    poi(PoiCategory::Other, 50),
];

/// The rule of a class, whatever the scale (`None`: never drawn).
pub fn rule(class: Class) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.class == class)
}

/// The rule for a feature of `layer` whose class attribute is `class`
/// (see [`Class::attribute`]), if it is drawn at `scale` m per pixel.
pub fn lookup(layer: &str, class: &str, scale: f64) -> Option<&'static Rule> {
    let layer = Layer::parse(layer)?;
    let class = Class::parse(layer, class)?;
    rule(class).filter(|r| r.visible(scale))
}

/// The rule for a boundary with this `admin_level`, if drawn at `scale`.
pub fn lookup_boundary(admin_level: i64, scale: f64) -> Option<&'static Rule> {
    let class = Class::Boundary(AdminLevel::from_level(admin_level)?);
    rule(class).filter(|r| r.visible(scale))
}

/// "always", "500 m", "2 km": a scale threshold for people.
pub fn scale_text(m: f64) -> String {
    if !m.is_finite() {
        "always".to_string()
    } else if m >= 1000.0 {
        format!("{} km", trim_number(m / 1000.0))
    } else {
        format!("{} m", trim_number(m))
    }
}

fn trim_number(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_string)
}

/// One line of the legend.
#[derive(Clone, Debug, PartialEq)]
pub struct LegendEntry {
    /// What it looks like on the map, [`LEGEND_SAMPLE_WIDTH`] cells drawn
    /// with the canvas itself.
    pub sample: Vec<Cell>,
    /// What it is, e.g. "Unclassified road, residential street" (ASCII).
    pub text: String,
    /// The table's classes it stands for (empty for the extra entries).
    pub classes: Vec<Class>,
    /// Shown when `min_scale < scale <= max_scale`.
    pub min_scale: f64,
    pub max_scale: f64,
}

impl LegendEntry {
    pub fn visible(&self, scale: f64) -> bool {
        at_most(scale, self.max_scale) && !at_most(scale, self.min_scale)
    }

    /// "always", "up to 500 m per pixel", "above 500 m per pixel".
    pub fn scales_text(&self) -> String {
        if self.min_scale > 0.0 {
            format!("above {} per pixel", scale_text(self.min_scale))
        } else if self.max_scale.is_finite() {
            format!("up to {} per pixel", scale_text(self.max_scale))
        } else {
            "always".to_string()
        }
    }
}

/// The legend: every rule of [`RULES`] (consecutive rows of one layer
/// drawn alike up to the same scale share a line), plus the sea, the
/// shading outside detailed maps, the coverage edge, street names and the
/// crosshair. With `Some(scale)`, only what is drawn at that scale.
pub fn legend(scale: Option<f64>) -> Vec<LegendEntry> {
    let mut out = Vec::new();
    let mut sea = sample_canvas();
    sea.fill(SEA);
    let mut e = entry(&sea, "Sea", 0.0, ALWAYS);
    // The background colour, and the sea areas of regional packs.
    e.classes = vec![Class::Water(WaterClass::Sea)];
    out.push(e);

    let mut i = 0;
    while i < RULES.len() {
        let first = &RULES[i];
        if first.class == Class::Water(WaterClass::Sea) {
            // On the "Sea" line above.
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < RULES.len() && same_look(first, &RULES[j]) {
            j += 1;
        }
        let words: Vec<&str> = RULES[i..j].iter().map(|r| r.class.description()).collect();
        let mut e = entry(
            &rule_sample(first),
            &capitalise(&words.join(", ")),
            0.0,
            first.max_scale,
        );
        e.classes = RULES[i..j].iter().map(|r| r.class).collect();
        out.push(e);
        i = j;
    }

    let mut street = sample_canvas();
    street.put_text(0, 0, "Name", Some(STREET_NAME_COLOUR), None, false);
    out.push(entry(
        &street,
        "Street name",
        0.0,
        STREET_NAMES.name_max_scale,
    ));
    let mut shade = sample_canvas();
    let mut all = CellMask::new(LEGEND_SAMPLE_WIDTH, 1);
    all.invert();
    shade.shade(&all, NO_DETAIL_SHADE);
    out.push(entry(
        &shade,
        "No detailed map here",
        0.0,
        NO_DETAIL_MAX_SCALE,
    ));
    let mut edge = sample_canvas();
    edge.line(
        (0.0, 0.0),
        (f64::from(LEGEND_SAMPLE_WIDTH), 0.0),
        COVERAGE_EDGE.0,
        COVERAGE_EDGE.1,
    );
    out.push(entry(
        &edge,
        "Edge of a detailed map",
        NO_DETAIL_MAX_SCALE,
        ALWAYS,
    ));
    let mut cross = sample_canvas();
    cross.put_char(0, 0, CROSSHAIR, None, None, true);
    out.push(entry(
        &cross,
        "Centre of the view (coordinates below)",
        0.0,
        ALWAYS,
    ));

    match scale {
        Some(s) => out.into_iter().filter(|e| e.visible(s)).collect(),
        None => out,
    }
}

fn sample_canvas() -> Canvas {
    Canvas::new(LEGEND_SAMPLE_WIDTH, 1)
}

/// Draw one rule the way the viewer does, on a one-row canvas.
fn rule_sample(rule: &Rule) -> Canvas {
    let mut c = sample_canvas();
    let w = f64::from(LEGEND_SAMPLE_WIDTH);
    match rule.draw {
        Draw::Fill => c.fill_polygon(&[[(0.0, 0.0), (w, 0.0), (w, 2.0), (0.0, 2.0)]], rule.colour),
        Draw::Line(p) => c.line((0.0, 0.0), (w, 0.0), rule.colour, p),
        Draw::Label(l) => {
            if let Some(m) = l.marker {
                c.put_char(0, 0, m, Some(rule.colour), None, false);
            }
        }
    }
    c
}

fn entry(c: &Canvas, text: &str, min_scale: f64, max_scale: f64) -> LegendEntry {
    LegendEntry {
        sample: c.cells(),
        text: text.to_string(),
        classes: Vec::new(),
        min_scale,
        max_scale,
    }
}

fn same_look(a: &Rule, b: &Rule) -> bool {
    let draw_alike = match (a.draw, b.draw) {
        (Draw::Fill, Draw::Fill) => true,
        (Draw::Line(p), Draw::Line(q)) => p == q,
        (Draw::Label(l), Draw::Label(m)) => l.marker == m.marker && l.priority == m.priority,
        _ => false,
    };
    a.class.layer() == b.class.layer()
        && a.colour == b.colour
        && a.max_scale == b.max_scale
        && draw_alike
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maps::canvas::is_console_char;

    /// The zoom ladder of ADR 0016 §3, in m per pixel.
    const LADDER: [f64; 15] = [
        5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1e3, 2e3, 5e3, 10e3, 20e3, 50e3, 100e3, 200e3,
    ];

    /// The ADR table, transcribed again independently of RULES: (layer,
    /// class, shown up to, colour, style).
    const ADR: &[(&str, &str, f64, Colour, &str)] = &[
        ("water", "lake", ALWAYS, Blue, "fill"),
        ("water", "coastline", 500.0, Blue, "line"),
        ("water", "river", 500.0, Blue, "line"),
        ("water", "stream", 50.0, Blue, "line"),
        ("water", "canal", 50.0, Blue, "line"),
        ("land", "forest", 500.0, Green, "fill"),
        ("land", "wood", 500.0, Green, "fill"),
        ("land", "park", 50.0, Green, "fill"),
        ("land", "wetland", 50.0, Green, "fill"),
        ("boundaries", "2", ALWAYS, Magenta, "dashed"),
        ("boundaries", "4", 5000.0, Magenta, "dashed"),
        ("boundaries", "6", 1000.0, Magenta, "dashed"),
        ("boundaries", "8", 100.0, Magenta, "dashed"),
        ("roads", "motorway", 2000.0, Red, "line"),
        ("roads", "trunk", 2000.0, Red, "line"),
        ("roads", "primary", 500.0, Yellow, "line"),
        ("roads", "secondary", 200.0, Yellow, "line"),
        ("roads", "tertiary", 100.0, White, "line"),
        ("roads", "unclassified", 20.0, White, "line"),
        ("roads", "residential", 20.0, White, "line"),
        ("roads", "service", 10.0, White, "dotted"),
        ("roads", "track", 10.0, White, "dotted"),
        ("roads", "path", 10.0, White, "dotted"),
        ("rail", "rail", 200.0, Cyan, "dashed"),
        ("places", "city", 20_000.0, White, "label"),
        ("places", "town", 2000.0, White, "label"),
        ("places", "village", 200.0, White, "label"),
        ("places", "hamlet", 50.0, White, "label"),
        ("peaks", "peak", 200.0, White, "label"),
        ("pois", "health", 20.0, Cyan, "label"),
        ("pois", "other", 20.0, Cyan, "label"),
    ];

    fn style_name(r: &Rule) -> &'static str {
        match r.draw {
            Draw::Fill => "fill",
            Draw::Line(Solid) => "line",
            Draw::Line(Dashed) => "dashed",
            Draw::Line(Dotted) => "dotted",
            Draw::Label(_) => "label",
        }
    }

    #[test]
    fn visibility_matches_the_adr_at_every_zoom_step() {
        for &(layer, class, up_to, colour, style) in ADR {
            for &s in &LADDER {
                let got = lookup(layer, class, s);
                assert_eq!(got.is_some(), s <= up_to, "{layer}/{class} at {s} m");
                if let Some(r) = got {
                    assert_eq!(r.colour, colour, "{layer}/{class}");
                    assert_eq!(style_name(r), style, "{layer}/{class}");
                }
            }
        }
        // Rounding near a step does not flip visibility.
        assert!(lookup("roads", "primary", 500.000_01).is_some());
        assert!(lookup("roads", "primary", 500.1).is_none());
        // Nonsense scales draw nothing.
        for s in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert!(lookup("water", "lake", s).is_none());
        }
    }

    #[test]
    fn label_details_follow_the_adr() {
        let poi = rule(Class::Poi(PoiCategory::Health))
            .and_then(Rule::label)
            .copied();
        let poi = poi.expect("poi rule");
        assert!(poi.name_visible(10.0) && !poi.name_visible(20.0));
        // The overview's spot heights are visible, named and with their
        // height at overview scales; a depression is not drawn as a peak.
        for (c, m) in [(PeakClass::Summit, '^'), (PeakClass::Depression, 'v')] {
            let r = rule(Class::Peak(c)).expect("spot height rule");
            assert!(r.visible(5_000.0) && !r.visible(20_000.0));
            let Draw::Label(l) = r.draw else {
                panic!("a label")
            };
            assert_eq!(l.marker, Some(m));
            assert!(l.name_visible(5_000.0) && l.detail_visible(5_000.0));
        }
        let peak = rule(Class::Peak(PeakClass::Peak))
            .and_then(Rule::label)
            .copied();
        let peak = peak.expect("peak rule");
        assert_eq!(peak.marker, Some('^'));
        assert!(peak.detail_visible(100.0) && !peak.detail_visible(200.0));
        assert!(STREET_NAMES.name_visible(5.0) && STREET_NAMES.name_visible(10.0));
        assert!(!STREET_NAMES.name_visible(20.0));
        // Priority: city > town > peak > village > POI > street name.
        let prio = |c: Class| rule(c).and_then(Rule::label).map_or(0, |l| l.priority);
        let order = [
            prio(Class::Place(PlaceClass::City)),
            prio(Class::Place(PlaceClass::Town)),
            prio(Class::Peak(PeakClass::Peak)),
            prio(Class::Place(PlaceClass::Village)),
            prio(Class::Place(PlaceClass::Hamlet)),
            prio(Class::Poi(PoiCategory::Health)),
            prio(Class::Poi(PoiCategory::Other)),
            STREET_NAMES.priority,
        ];
        assert!(order.windows(2).all(|w| w[0] > w[1]), "{order:?}");
        // POI letters are distinct, printable and not the crosshair or a
        // place/peak marker.
        let mut letters: Vec<char> = PoiCategory::ALL.iter().map(|c| c.letter()).collect();
        letters.sort_unstable();
        letters.dedup();
        assert_eq!(letters.len(), PoiCategory::ALL.len());
        assert!(letters.iter().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn names_round_trip_and_every_class_has_one_rule() {
        for l in Layer::ALL {
            assert_eq!(Layer::parse(l.as_str()), Some(*l));
        }
        assert_eq!(Layer::parse("Water"), None);
        let mut seen = std::collections::HashSet::new();
        for c in Class::all() {
            assert_eq!(Class::parse(c.layer(), c.as_str()), Some(c));
            assert!(c.description().is_ascii());
            let n = RULES.iter().filter(|r| r.class == c).count();
            let expected = usize::from(c != Class::Land(LandClass::Farmland));
            assert_eq!(n, expected, "{c:?}");
            assert!(seen.insert((c.layer(), c.as_str())));
        }
        assert_eq!(RULES.len(), Class::all().count() - 1);
        assert_eq!(lookup("boundaries", "3", 1.0), None);
        assert_eq!(
            lookup_boundary(2, 1e9),
            rule(Class::Boundary(AdminLevel::Country))
        );
        assert_eq!(lookup_boundary(8, 200.0), None);
        assert_eq!(lookup_boundary(-1, 5.0), None);
        assert_eq!(lookup("roads", "highway", 5.0), None);
        assert_eq!(lookup("land", "farmland", 5.0), None);
        assert_eq!(Class::attribute(Layer::Boundaries), "admin_level");
    }

    #[test]
    fn passes_and_ranks() {
        // Areas and lines in RULES are in drawing order; labels follow
        // them, by priority (placement order, not drawing order).
        let keys: Vec<_> = RULES
            .iter()
            .filter(|r| r.pass() != Pass::Labels)
            .map(Rule::sort_key)
            .collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
        let first_label = RULES.iter().position(|r| r.pass() == Pass::Labels);
        let first_label = first_label.unwrap_or(RULES.len());
        assert!(
            RULES[first_label..]
                .iter()
                .all(|r| r.pass() == Pass::Labels)
        );
        let prios: Vec<u16> = RULES[first_label..]
            .iter()
            .filter_map(|r| r.label().map(|l| l.priority))
            .collect();
        assert!(prios.windows(2).all(|w| w[0] >= w[1]), "{prios:?}");
        let key = |c| rule(c).map(Rule::sort_key);
        assert!(key(Class::Water(WaterClass::Lake)) > key(Class::Land(LandClass::Forest)));
        assert!(key(Class::Road(RoadClass::Motorway)) > key(Class::Road(RoadClass::Residential)));
        assert!(key(Class::Boundary(AdminLevel::Country)) > key(Class::Road(RoadClass::Motorway)));
        assert_eq!(
            rule(Class::Land(LandClass::Park)).map(Rule::pass),
            Some(Pass::Areas)
        );
        assert_eq!(
            rule(Class::Place(PlaceClass::City)).map(Rule::pass),
            Some(Pass::Labels)
        );
    }

    #[test]
    fn min_zoom_for_the_builder() {
        let z = |c| rule(c).map(Rule::min_zoom);
        assert_eq!(z(Class::Boundary(AdminLevel::Country)), Some(0));
        assert_eq!(z(Class::Place(PlaceClass::City)), Some(2));
        assert_eq!(z(Class::Road(RoadClass::Motorway)), Some(5));
        assert_eq!(z(Class::Road(RoadClass::Primary)), Some(7));
        assert_eq!(z(Class::Road(RoadClass::Tertiary)), Some(10));
        assert_eq!(z(Class::Road(RoadClass::Residential)), Some(12));
        assert_eq!(z(Class::Road(RoadClass::Path)), Some(13));
        assert_eq!(min_zoom_for_scale(5.0), 14);
        assert_eq!(min_zoom_for_scale(0.01), 14);
        assert_eq!(min_zoom_for_scale(1e9), 0);
        assert_eq!(min_zoom_for_scale(f64::NAN), 0);
        // The builder can iterate over every class with its zoom.
        assert!(RULES.iter().all(|r| r.min_zoom() <= MAX_TILE_ZOOM));
    }

    #[test]
    fn legend_agrees_with_the_table() {
        let all = legend(None);
        // Every rule is in exactly one entry, with its scale, its words
        // and its colour.
        for r in RULES {
            let hits: Vec<_> = all
                .iter()
                .filter(|e| e.classes.contains(&r.class))
                .collect();
            assert_eq!(hits.len(), 1, "{:?}", r.class);
            let e = hits[0];
            assert_eq!(e.max_scale, r.max_scale, "{:?}", r.class);
            assert!(e.text.to_ascii_lowercase().contains(r.class.description()));
            assert!(
                e.sample
                    .iter()
                    .any(|c| c.fg == Some(r.colour) || c.bg == Some(r.colour)),
                "{:?}",
                r.class
            );
        }
        // At each step the legend lists something exactly when the table
        // draws it, and its sample has the rule's colour.
        for &s in &LADDER {
            let shown = legend(Some(s));
            for r in RULES {
                let in_legend = shown.iter().any(|e| e.classes.contains(&r.class));
                assert_eq!(in_legend, r.visible(s), "{:?} at {s}", r.class);
            }
            let shading = shown.iter().any(|e| e.text.starts_with("No detailed"));
            let edge = shown.iter().any(|e| e.text.starts_with("Edge"));
            assert_eq!(shading, s <= NO_DETAIL_MAX_SCALE);
            assert_eq!(edge, !shading);
        }
        let poi = all
            .iter()
            .find(|e| e.text.starts_with("Health"))
            .expect("health");
        assert_eq!(poi.sample[0].ch, 'H');
        assert_eq!(poi.scales_text(), "up to 20 m per pixel");
        let red = all
            .iter()
            .find(|e| e.classes.contains(&Class::Road(RoadClass::Motorway)))
            .expect("motorway");
        assert_eq!(red.text, "Trunk road, motorway");
        assert_eq!(red.scales_text(), "up to 2 km per pixel");
        let road = all
            .iter()
            .find(|e| e.text.starts_with("Unclassified"))
            .expect("roads");
        assert_eq!(road.text, "Unclassified road, residential street");
        let border = all
            .iter()
            .find(|e| e.text.starts_with("Country"))
            .expect("border");
        assert_eq!(border.scales_text(), "always");
        assert_eq!(scale_text(2000.0), "2 km");
        assert_eq!(scale_text(500.0), "500 m");
        assert_eq!(scale_text(2500.0), "2.5 km");
        for e in &all {
            assert_eq!(e.sample.len(), usize::from(LEGEND_SAMPLE_WIDTH));
            assert!(
                e.text.chars().all(|c| c == ' ' || c.is_ascii_graphic()),
                "{}",
                e.text
            );
            assert!(e.sample.iter().all(|c| is_console_char(c.ch)));
        }
    }
}
