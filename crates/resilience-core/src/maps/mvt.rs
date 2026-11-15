//! A bounded decoder for Mapbox Vector Tiles 2.1 (ADR 0016 §7).
//!
//! Input is the bytes of one tile, **already decompressed** (the gzip
//! limit lives with the pack reader), and it is untrusted: a map file on a
//! stick may have been made by anyone. The decoder is written by hand from
//! the protobuf wire format (varints and length-delimited fields; no
//! protobuf crate) and enforces every bound of the ADR as a named constant
//! below. Malformed input gives an [`MvtError`], never a panic, and work
//! and memory are linear in the input size, which is itself bounded by
//! [`MAX_TILE_BYTES`].
//!
//! Policy for things the format allows but we do not draw:
//! - **Unknown fields** are skipped according to their wire type, at every
//!   level (tile, layer, feature, value). Group wire types (3, 4) and the
//!   undefined ones (6, 7) are errors.
//! - **A layer whose version is not 2** (including a missing version, which
//!   protobuf reads as the default 1) is decoded and checked like any
//!   other, then left out of [`Tile::layers`] and counted in
//!   [`Tile::skipped_layers`]. Version 1 had no winding-order rule, so its
//!   polygons cannot be filled correctly; later versions are unknown.
//! - **A feature whose geometry type is UNKNOWN (0) or not one of the three
//!   defined** is left out and counted in [`Layer::skipped_features`]; its
//!   tags and geometry still count towards the bounds. The specification
//!   allows decoders to ignore such features.
//! - **A polygon ring of zero area** is dropped: it has neither the
//!   exterior nor the interior winding, and fills nothing.
//! - Everything else that breaks the specification (a tag index out of
//!   range, an odd number of tags, a malformed command sequence, two layers
//!   with the same name, a value with no or several fields, invalid UTF-8)
//!   rejects the whole tile: a valid tile never needs these, and a partial
//!   decode of a hostile one would only hide the problem.
//!
//! The decoded form is meant for the renderer: each layer keeps all its
//! vertices in one `Vec<Point>` and its geometry parts as ranges into it,
//! so iterating a feature allocates nothing, and decoding allocates per
//! part and feature only through amortised growth of a few vectors.

use thiserror::Error;

/// Largest decompressed tile accepted (ADR: decompressed 8 MiB).
pub const MAX_TILE_BYTES: usize = 8 * 1024 * 1024;
/// Most layers in a tile, including skipped ones.
pub const MAX_LAYERS: usize = 64;
/// Most keys in a tile, all layers together.
pub const MAX_KEYS: usize = 65_536;
/// Most values in a tile, all layers together.
pub const MAX_VALUES: usize = 65_536;
/// Most features in a tile, all layers together, including skipped ones.
pub const MAX_FEATURES: usize = 200_000;
/// Most geometry integers (commands and parameters) in a tile.
pub const MAX_GEOMETRY_INTEGERS: usize = 2_000_000;
/// Most tag integers in a tile. Not in the ADR's list: added so that every
/// repeated field has a bound of its own, not only the input size.
pub const MAX_TAG_INTEGERS: usize = 2_000_000;
/// Longest string (layer name, key, string value), in bytes.
pub const MAX_STRING_BYTES: usize = 1024;
/// Smallest accepted layer extent.
pub const MIN_EXTENT: u32 = 256;
/// Largest accepted layer extent.
pub const MAX_EXTENT: u32 = 65_536;
/// Extent of a layer that does not state one (MVT default).
pub const DEFAULT_EXTENT: u32 = 4096;
/// The only layer version that is kept.
pub const SUPPORTED_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MvtError {
    #[error("the tile is larger than {MAX_TILE_BYTES} bytes")]
    TooLarge,
    #[error("the tile ends in the middle of a field")]
    Truncated,
    #[error("a number is encoded with too many bytes")]
    BadVarint,
    #[error("a number is too large for its field")]
    NumberTooLarge,
    #[error("a field number is invalid")]
    BadFieldNumber,
    #[error("unsupported protobuf wire type {0}")]
    BadWireType(u8),
    #[error("field {field} has the wrong wire type")]
    WrongWireType { field: u32 },
    #[error("more than {MAX_LAYERS} layers")]
    TooManyLayers,
    #[error("more than {MAX_KEYS} keys")]
    TooManyKeys,
    #[error("more than {MAX_VALUES} values")]
    TooManyValues,
    #[error("more than {MAX_FEATURES} features")]
    TooManyFeatures,
    #[error("more than {MAX_GEOMETRY_INTEGERS} geometry integers")]
    TooManyGeometryIntegers,
    #[error("more than {MAX_TAG_INTEGERS} tag integers")]
    TooManyTagIntegers,
    #[error("a string is longer than {MAX_STRING_BYTES} bytes")]
    StringTooLong,
    #[error("a string is not valid UTF-8")]
    BadUtf8,
    #[error("layer extent {0} is outside {MIN_EXTENT}..={MAX_EXTENT}")]
    BadExtent(u64),
    #[error("a layer has no name")]
    MissingLayerName,
    #[error("two layers have the same name")]
    DuplicateLayerName,
    #[error("a value must have exactly one field")]
    BadValue,
    #[error("a feature has an odd number of tags")]
    OddTags,
    #[error("a tag refers to a key or value that does not exist")]
    TagOutOfRange,
    #[error("malformed geometry: {0}")]
    BadGeometry(&'static str),
    #[error("a coordinate is out of range")]
    CoordinateOutOfRange,
}

type Result<T> = std::result::Result<T, MvtError>;

// ---------------------------------------------------------------------------
// Decoded tile
// ---------------------------------------------------------------------------

/// A vertex in tile coordinates: `0..extent` inside the tile, `y` down.
/// Buffer geometry may lie outside, by any amount within `i32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeomType {
    Point,
    LineString,
    Polygon,
}

/// What a geometry part is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartRole {
    /// All the points of a (multi)point feature.
    Points,
    /// One line string.
    Line,
    /// A polygon's outer ring (positive area in tile coordinates). The
    /// closing vertex is implied, not repeated.
    Exterior,
    /// A hole of the exterior ring before it (negative area).
    Interior,
}

/// A value of a feature attribute.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Float(f32),
    Double(f64),
    Int(i64),
    Uint(u64),
    Sint(i64),
    Bool(bool),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Any number as `f64`; `None` for strings, booleans, NaN and
    /// infinities.
    pub fn as_f64(&self) -> Option<f64> {
        let v = match *self {
            Value::Float(v) => f64::from(v),
            Value::Double(v) => v,
            Value::Int(v) | Value::Sint(v) => v as f64,
            Value::Uint(v) => v as f64,
            Value::String(_) | Value::Bool(_) => return None,
        };
        v.is_finite().then_some(v)
    }

    /// An integer, or a float with no fractional part that fits an `i64`
    /// (tile builders write `admin_level` or `ele` either way).
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Int(v) | Value::Sint(v) => Some(v),
            Value::Uint(v) => i64::try_from(v).ok(),
            Value::Float(_) | Value::Double(_) => {
                let v = self.as_f64()?;
                // -2^63 is exact as f64; integral values in [-2^63, 2^63)
                // convert exactly.
                let min = i64::MIN as f64;
                (v.fract() == 0.0 && v >= min && v < -min).then_some(v as i64)
            }
            Value::String(_) | Value::Bool(_) => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PartData {
    start: u32,
    end: u32,
    role: PartRole,
}

#[derive(Debug, Clone, Copy)]
struct FeatureData {
    id: Option<u64>,
    geom_type: GeomType,
    tags_start: u32,
    tags_end: u32,
    parts_start: u32,
    parts_end: u32,
}

/// A decoded tile: its layers of version 2, in file order, with unique
/// names.
#[derive(Debug, Clone, Default)]
pub struct Tile {
    layers: Vec<Layer>,
    skipped_layers: usize,
}

impl Tile {
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }

    /// Layers left out because their version is not [`SUPPORTED_VERSION`].
    pub fn skipped_layers(&self) -> usize {
        self.skipped_layers
    }
}

/// One layer of a tile. Every tag of every feature was checked against
/// `keys` and `values`.
#[derive(Debug, Clone)]
pub struct Layer {
    name: String,
    extent: u32,
    keys: Vec<String>,
    values: Vec<Value>,
    features: Vec<FeatureData>,
    parts: Vec<PartData>,
    points: Vec<Point>,
    tags: Vec<u32>,
    skipped_features: usize,
}

impl Layer {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Tile coordinates run over `0..extent` (between [`MIN_EXTENT`] and
    /// [`MAX_EXTENT`]).
    pub fn extent(&self) -> u32 {
        self.extent
    }

    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// Number of features kept.
    pub fn len(&self) -> usize {
        self.features.len()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    /// Features left out because their geometry type is unknown.
    pub fn skipped_features(&self) -> usize {
        self.skipped_features
    }

    /// Total vertices of the layer (for cache accounting).
    pub fn vertex_count(&self) -> usize {
        self.points.len()
    }

    pub fn features(&self) -> impl ExactSizeIterator<Item = Feature<'_>> + '_ {
        self.features
            .iter()
            .map(move |data| Feature { layer: self, data })
    }

    pub fn feature(&self, index: usize) -> Option<Feature<'_>> {
        self.features
            .get(index)
            .map(|data| Feature { layer: self, data })
    }

    /// The index of `key` among the layer's keys, for repeated lookups
    /// with [`Feature::attr_at`] without comparing strings.
    pub fn key_index(&self, key: &str) -> Option<u32> {
        self.keys
            .iter()
            .position(|k| k == key)
            .and_then(|i| u32::try_from(i).ok())
    }
}

/// A feature of a [`Layer`], borrowed.
#[derive(Debug, Clone, Copy)]
pub struct Feature<'a> {
    layer: &'a Layer,
    data: &'a FeatureData,
}

/// One part of a feature's geometry: a slice of the layer's vertices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part<'a> {
    pub role: PartRole,
    pub points: &'a [Point],
}

impl<'a> Feature<'a> {
    pub fn id(&self) -> Option<u64> {
        self.data.id
    }

    pub fn geom_type(&self) -> GeomType {
        self.data.geom_type
    }

    /// Geometry parts in file order. A polygon is a sequence of exterior
    /// rings, each followed by its interior rings.
    pub fn parts(&self) -> impl ExactSizeIterator<Item = Part<'a>> + 'a {
        let layer = self.layer;
        let parts = layer
            .parts
            .get(self.data.parts_start as usize..self.data.parts_end as usize)
            .unwrap_or(&[]);
        parts.iter().map(move |p| Part {
            role: p.role,
            points: layer
                .points
                .get(p.start as usize..p.end as usize)
                .unwrap_or(&[]),
        })
    }

    fn tag_pairs(&self) -> impl Iterator<Item = (u32, u32)> + 'a {
        let tags = self
            .layer
            .tags
            .get(self.data.tags_start as usize..self.data.tags_end as usize)
            .unwrap_or(&[]);
        tags.chunks_exact(2).map(|kv| (kv[0], kv[1]))
    }

    /// The value of attribute `key` (the first one, if a broken tile
    /// repeats it).
    pub fn attr(&self, key: &str) -> Option<&'a Value> {
        let layer = self.layer;
        self.tag_pairs()
            .find(|&(k, _)| layer.keys.get(k as usize).is_some_and(|s| s == key))
            .and_then(|(_, v)| layer.values.get(v as usize))
    }

    /// The value of the attribute whose key index is `key` (from
    /// [`Layer::key_index`]).
    pub fn attr_at(&self, key: u32) -> Option<&'a Value> {
        self.tag_pairs()
            .find(|&(k, _)| k == key)
            .and_then(|(_, v)| self.layer.values.get(v as usize))
    }

    /// All attributes, in file order.
    pub fn attrs(&self) -> impl Iterator<Item = (&'a str, &'a Value)> + 'a {
        let layer = self.layer;
        self.tag_pairs().filter_map(move |(k, v)| {
            Some((
                layer.keys.get(k as usize)?.as_str(),
                layer.values.get(v as usize)?,
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Protobuf wire format
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
    Varint,
    Fixed64,
    Len,
    Fixed32,
}

/// Highest protobuf field number.
const MAX_FIELD_NUMBER: u64 = (1 << 29) - 1;

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// A base-128 varint of at most 10 bytes; the 10th may only hold the
    /// 64th bit.
    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for i in 0..10 {
            let byte = *self.buf.get(self.pos).ok_or(MvtError::Truncated)?;
            self.pos += 1;
            if i == 9 && byte > 1 {
                return Err(MvtError::BadVarint);
            }
            value |= u64::from(byte & 0x7f) << (7 * i);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(MvtError::BadVarint)
    }

    fn varint_u32(&mut self) -> Result<u32> {
        u32::try_from(self.varint()?).map_err(|_| MvtError::NumberTooLarge)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let end = self.pos.checked_add(N).ok_or(MvtError::Truncated)?;
        let bytes = self.buf.get(self.pos..end).ok_or(MvtError::Truncated)?;
        self.pos = end;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = usize::try_from(self.varint()?).map_err(|_| MvtError::Truncated)?;
        let end = self.pos.checked_add(len).ok_or(MvtError::Truncated)?;
        let bytes = self.buf.get(self.pos..end).ok_or(MvtError::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }

    fn key(&mut self) -> Result<(u32, Wire)> {
        let key = self.varint()?;
        let field = key >> 3;
        if field == 0 || field > MAX_FIELD_NUMBER {
            return Err(MvtError::BadFieldNumber);
        }
        let wire = match key & 7 {
            0 => Wire::Varint,
            1 => Wire::Fixed64,
            2 => Wire::Len,
            5 => Wire::Fixed32,
            other => return Err(MvtError::BadWireType(other as u8)),
        };
        Ok((field as u32, wire))
    }

    fn skip(&mut self, wire: Wire) -> Result<()> {
        match wire {
            Wire::Varint => self.varint().map(drop),
            Wire::Fixed64 => self.fixed::<8>().map(drop),
            Wire::Fixed32 => self.fixed::<4>().map(drop),
            Wire::Len => self.bytes().map(drop),
        }
    }
}

fn string(bytes: &[u8]) -> Result<String> {
    if bytes.len() > MAX_STRING_BYTES {
        return Err(MvtError::StringTooLong);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| MvtError::BadUtf8)
}

fn zigzag32(n: u32) -> i32 {
    ((n >> 1) as i32) ^ -((n & 1) as i32)
}

fn zigzag64(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

/// What is left of each per-tile bound.
struct Budget {
    layers: usize,
    keys: usize,
    values: usize,
    features: usize,
    geometry: usize,
    tags: usize,
}

fn take(left: &mut usize, n: usize, err: MvtError) -> Result<()> {
    *left = left.checked_sub(n).ok_or(err)?;
    Ok(())
}

/// Append a repeated `uint32` field (packed or not) to `out`, charging
/// `left`.
fn repeated_u32(
    r: &mut Reader<'_>,
    wire: Wire,
    field: u32,
    out: &mut Vec<u32>,
    left: &mut usize,
    err: MvtError,
) -> Result<()> {
    match wire {
        Wire::Len => {
            let mut packed = Reader::new(r.bytes()?);
            while !packed.at_end() {
                take(left, 1, err)?;
                out.push(packed.varint_u32()?);
            }
            Ok(())
        }
        Wire::Varint => {
            take(left, 1, err)?;
            out.push(r.varint_u32()?);
            Ok(())
        }
        _ => Err(MvtError::WrongWireType { field }),
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decode one tile (already decompressed).
pub fn decode(bytes: &[u8]) -> Result<Tile> {
    if bytes.len() > MAX_TILE_BYTES {
        return Err(MvtError::TooLarge);
    }
    let mut budget = Budget {
        layers: MAX_LAYERS,
        keys: MAX_KEYS,
        values: MAX_VALUES,
        features: MAX_FEATURES,
        geometry: MAX_GEOMETRY_INTEGERS,
        tags: MAX_TAG_INTEGERS,
    };
    let mut geometry = Vec::new();
    let mut tile = Tile::default();
    let mut r = Reader::new(bytes);
    while !r.at_end() {
        match r.key()? {
            (3, Wire::Len) => {
                take(&mut budget.layers, 1, MvtError::TooManyLayers)?;
                let (layer, version) = decode_layer(r.bytes()?, &mut budget, &mut geometry)?;
                if version != u64::from(SUPPORTED_VERSION) {
                    tile.skipped_layers += 1;
                } else if tile.layers.iter().any(|l| l.name == layer.name) {
                    return Err(MvtError::DuplicateLayerName);
                } else {
                    tile.layers.push(layer);
                }
            }
            (3, _) => return Err(MvtError::WrongWireType { field: 3 }),
            (_, wire) => r.skip(wire)?,
        }
    }
    Ok(tile)
}

/// One layer and its version. `geometry` is scratch space reused across
/// features.
fn decode_layer(
    bytes: &[u8],
    budget: &mut Budget,
    geometry: &mut Vec<u32>,
) -> Result<(Layer, u64)> {
    let mut layer = Layer {
        name: String::new(),
        extent: DEFAULT_EXTENT,
        keys: Vec::new(),
        values: Vec::new(),
        features: Vec::new(),
        parts: Vec::new(),
        points: Vec::new(),
        tags: Vec::new(),
        skipped_features: 0,
    };
    let mut version = 1u64;
    let mut extent = u64::from(DEFAULT_EXTENT);
    let mut r = Reader::new(bytes);
    while !r.at_end() {
        match r.key()? {
            (1, Wire::Len) => layer.name = string(r.bytes()?)?,
            (2, Wire::Len) => {
                take(&mut budget.features, 1, MvtError::TooManyFeatures)?;
                decode_feature(r.bytes()?, &mut layer, budget, geometry)?;
            }
            (3, Wire::Len) => {
                take(&mut budget.keys, 1, MvtError::TooManyKeys)?;
                let key = string(r.bytes()?)?;
                layer.keys.push(key);
            }
            (4, Wire::Len) => {
                take(&mut budget.values, 1, MvtError::TooManyValues)?;
                let value = decode_value(r.bytes()?)?;
                layer.values.push(value);
            }
            (5, Wire::Varint) => extent = r.varint()?,
            (15, Wire::Varint) => version = r.varint()?,
            (field @ (1..=5 | 15), _) => return Err(MvtError::WrongWireType { field }),
            (_, wire) => r.skip(wire)?,
        }
    }
    if layer.name.is_empty() {
        return Err(MvtError::MissingLayerName);
    }
    layer.extent = u32::try_from(extent)
        .ok()
        .filter(|e| (MIN_EXTENT..=MAX_EXTENT).contains(e))
        .ok_or(MvtError::BadExtent(extent))?;
    // Keys and values may come after the features: check tags at the end.
    let (nkeys, nvalues) = (layer.keys.len(), layer.values.len());
    for kv in layer.tags.chunks_exact(2) {
        if kv[0] as usize >= nkeys || kv[1] as usize >= nvalues {
            return Err(MvtError::TagOutOfRange);
        }
    }
    Ok((layer, version))
}

fn index(n: usize) -> Result<u32> {
    // Unreachable with the bounds above; checked rather than assumed.
    u32::try_from(n).map_err(|_| MvtError::TooLarge)
}

fn decode_feature(
    bytes: &[u8],
    layer: &mut Layer,
    budget: &mut Budget,
    geometry: &mut Vec<u32>,
) -> Result<()> {
    let mut id = None;
    let mut geom_type = 0u64;
    let tags_start = layer.tags.len();
    geometry.clear();
    let mut r = Reader::new(bytes);
    while !r.at_end() {
        match r.key()? {
            (1, Wire::Varint) => id = Some(r.varint()?),
            (2, wire) => repeated_u32(
                &mut r,
                wire,
                2,
                &mut layer.tags,
                &mut budget.tags,
                MvtError::TooManyTagIntegers,
            )?,
            (3, Wire::Varint) => geom_type = r.varint()?,
            (4, wire) => repeated_u32(
                &mut r,
                wire,
                4,
                geometry,
                &mut budget.geometry,
                MvtError::TooManyGeometryIntegers,
            )?,
            (field @ (1 | 3), _) => return Err(MvtError::WrongWireType { field }),
            (_, wire) => r.skip(wire)?,
        }
    }
    let geom_type = match geom_type {
        1 => GeomType::Point,
        2 => GeomType::LineString,
        3 => GeomType::Polygon,
        _ => {
            layer.tags.truncate(tags_start);
            layer.skipped_features += 1;
            return Ok(());
        }
    };
    if (layer.tags.len() - tags_start) % 2 != 0 {
        return Err(MvtError::OddTags);
    }
    let parts_start = layer.parts.len();
    decode_geometry(geom_type, geometry, &mut layer.points, &mut layer.parts)?;
    layer.features.push(FeatureData {
        id,
        geom_type,
        tags_start: index(tags_start)?,
        tags_end: index(layer.tags.len())?,
        parts_start: index(parts_start)?,
        parts_end: index(layer.parts.len())?,
    });
    Ok(())
}

fn decode_value(bytes: &[u8]) -> Result<Value> {
    let mut value = None;
    let mut fields = 0usize;
    let mut r = Reader::new(bytes);
    while !r.at_end() {
        let v = match r.key()? {
            (1, Wire::Len) => Value::String(string(r.bytes()?)?),
            (2, Wire::Fixed32) => Value::Float(f32::from_le_bytes(r.fixed::<4>()?)),
            (3, Wire::Fixed64) => Value::Double(f64::from_le_bytes(r.fixed::<8>()?)),
            (4, Wire::Varint) => Value::Int(r.varint()? as i64),
            (5, Wire::Varint) => Value::Uint(r.varint()?),
            (6, Wire::Varint) => Value::Sint(zigzag64(r.varint()?)),
            (7, Wire::Varint) => Value::Bool(r.varint()? != 0),
            (field @ 1..=7, _) => return Err(MvtError::WrongWireType { field }),
            (_, wire) => {
                r.skip(wire)?;
                continue;
            }
        };
        fields += 1;
        value = Some(v);
    }
    match value {
        Some(v) if fields == 1 => Ok(v),
        _ => Err(MvtError::BadValue),
    }
}

const MOVE_TO: u32 = 1;
const LINE_TO: u32 = 2;
const CLOSE_PATH: u32 = 7;

/// Walks a feature's geometry integers, keeping the cursor.
struct GeomReader<'a> {
    ints: &'a [u32],
    pos: usize,
    x: i64,
    y: i64,
}

impl GeomReader<'_> {
    fn at_end(&self) -> bool {
        self.pos >= self.ints.len()
    }

    /// The next command, if it is `id`: its count, after checking that the
    /// parameters are there (so a huge count costs nothing).
    fn command(&mut self, id: u32, what: &'static str) -> Result<usize> {
        let cmd = *self.ints.get(self.pos).ok_or(MvtError::BadGeometry(what))?;
        if cmd & 7 != id {
            return Err(MvtError::BadGeometry(what));
        }
        let count = (cmd >> 3) as usize;
        let params = if id == CLOSE_PATH { 0 } else { 2 };
        let needed = count
            .checked_mul(params)
            .ok_or(MvtError::BadGeometry(what))?;
        if needed > self.ints.len() - self.pos - 1 {
            return Err(MvtError::BadGeometry("parameters missing"));
        }
        self.pos += 1;
        Ok(count)
    }

    fn point(&mut self) -> Result<Point> {
        let (dx, dy) = match self.ints.get(self.pos..self.pos + 2) {
            Some(&[dx, dy]) => (zigzag32(dx), zigzag32(dy)),
            _ => return Err(MvtError::BadGeometry("parameters missing")),
        };
        self.pos += 2;
        self.x = self
            .x
            .checked_add(i64::from(dx))
            .ok_or(MvtError::CoordinateOutOfRange)?;
        self.y = self
            .y
            .checked_add(i64::from(dy))
            .ok_or(MvtError::CoordinateOutOfRange)?;
        Ok(Point {
            x: i32::try_from(self.x).map_err(|_| MvtError::CoordinateOutOfRange)?,
            y: i32::try_from(self.y).map_err(|_| MvtError::CoordinateOutOfRange)?,
        })
    }

    fn points(&mut self, count: usize, out: &mut Vec<Point>) -> Result<()> {
        for _ in 0..count {
            let p = self.point()?;
            out.push(p);
        }
        Ok(())
    }
}

/// Twice the signed area of a ring (surveyor's formula, closing edge
/// included), exactly: i128 cannot overflow for a million i32 vertices.
fn ring_area2(ring: &[Point]) -> i128 {
    let mut sum = 0i128;
    for (i, a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        sum += i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y);
    }
    sum
}

fn decode_geometry(
    geom_type: GeomType,
    ints: &[u32],
    points: &mut Vec<Point>,
    parts: &mut Vec<PartData>,
) -> Result<()> {
    if ints.is_empty() {
        return Err(MvtError::BadGeometry("no geometry"));
    }
    let mut g = GeomReader {
        ints,
        pos: 0,
        x: 0,
        y: 0,
    };
    match geom_type {
        GeomType::Point => {
            let count = g.command(MOVE_TO, "a point needs one MoveTo")?;
            if count == 0 {
                return Err(MvtError::BadGeometry("MoveTo without points"));
            }
            let start = index(points.len())?;
            g.points(count, points)?;
            if !g.at_end() {
                return Err(MvtError::BadGeometry("a point has extra commands"));
            }
            parts.push(PartData {
                start,
                end: index(points.len())?,
                role: PartRole::Points,
            });
        }
        GeomType::LineString => {
            while !g.at_end() {
                let start = index(points.len())?;
                if g.command(MOVE_TO, "a line starts with MoveTo")? != 1 {
                    return Err(MvtError::BadGeometry("a line's MoveTo has one point"));
                }
                g.points(1, points)?;
                let count = g.command(LINE_TO, "a line needs LineTo after MoveTo")?;
                if count == 0 {
                    return Err(MvtError::BadGeometry("LineTo without points"));
                }
                g.points(count, points)?;
                parts.push(PartData {
                    start,
                    end: index(points.len())?,
                    role: PartRole::Line,
                });
            }
        }
        GeomType::Polygon => {
            while !g.at_end() {
                let start = points.len();
                if g.command(MOVE_TO, "a ring starts with MoveTo")? != 1 {
                    return Err(MvtError::BadGeometry("a ring's MoveTo has one point"));
                }
                g.points(1, points)?;
                let count = g.command(LINE_TO, "a ring needs LineTo after MoveTo")?;
                if count < 2 {
                    return Err(MvtError::BadGeometry("a ring needs at least three points"));
                }
                g.points(count, points)?;
                if g.command(CLOSE_PATH, "a ring ends with ClosePath")? != 1 {
                    return Err(MvtError::BadGeometry("ClosePath takes a count of one"));
                }
                let area = ring_area2(&points[start..]);
                let role = match area.signum() {
                    1 => PartRole::Exterior,
                    -1 => PartRole::Interior,
                    _ => {
                        points.truncate(start);
                        continue;
                    }
                };
                parts.push(PartData {
                    start: index(start)?,
                    end: index(points.len())?,
                    role,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::mvt_encode::*;
    use super::*;

    fn pts(points: &[Point]) -> Vec<(i32, i32)> {
        points.iter().map(|p| (p.x, p.y)).collect()
    }

    fn one_layer(layer: &LayerBuilder) -> Vec<u8> {
        tile(&[layer.encode()])
    }

    /// A tile using every geometry type and value type, for the mutation
    /// tests.
    fn sample_tile() -> Vec<u8> {
        let mut roads = LayerBuilder::new("roads");
        roads.keys = vec!["class".into(), "name".into(), "rank".into()];
        roads.values = vec![
            value(&TestValue::Str("primary")),
            value(&TestValue::Str("Rue de l'Essai")),
            value(&TestValue::Sint(-3)),
            value(&TestValue::Double(2.5)),
            value(&TestValue::Float(1.0)),
            value(&TestValue::Int(-7)),
            value(&TestValue::Uint(9)),
            value(&TestValue::Bool(true)),
        ];
        roads.features.push(feature(
            Some(1),
            2,
            &[0, 0, 1, 1, 2, 2],
            &lines(&[&[(0, 0), (100, 50), (4096, 4096)], &[(10, 10), (20, 30)]]),
        ));
        roads.features.push(feature(
            None,
            3,
            &[0, 3, 2, 4],
            &polygon(&[
                &[(0, 0), (100, 0), (100, 100), (0, 100)],
                &[(10, 10), (10, 90), (90, 90), (90, 10)],
            ]),
        ));
        roads.features.push(feature(
            Some(3),
            1,
            &[2, 5, 1, 7],
            &points(&[(5, 5), (-20, 4200)]),
        ));
        let mut places = LayerBuilder::new("places");
        places.extent = Some(512);
        places.keys = vec!["name".into()];
        places.values = vec![
            value(&TestValue::Str("Testville")),
            value(&TestValue::Uint(6)),
        ];
        places
            .features
            .push(feature(None, 1, &[0, 0], &points(&[(256, 256)])));
        tile(&[roads.encode(), places.encode()])
    }

    #[test]
    fn decodes_the_sample_tile() {
        let t = decode(&sample_tile()).expect("valid tile");
        assert_eq!(t.layers().len(), 2);
        assert_eq!(t.skipped_layers(), 0);
        let roads = t.layer("roads").expect("roads");
        assert_eq!((roads.extent(), roads.len()), (DEFAULT_EXTENT, 3));
        let line = roads.feature(0).expect("line");
        assert_eq!(
            (line.id(), line.geom_type()),
            (Some(1), GeomType::LineString)
        );
        let parts: Vec<_> = line.parts().map(|p| (p.role, pts(p.points))).collect();
        assert_eq!(
            parts,
            [
                (PartRole::Line, vec![(0, 0), (100, 50), (4096, 4096)]),
                (PartRole::Line, vec![(10, 10), (20, 30)]),
            ]
        );
        assert_eq!(line.attr("class").and_then(Value::as_str), Some("primary"));
        assert_eq!(
            line.attr("name").and_then(Value::as_str),
            Some("Rue de l'Essai")
        );
        assert_eq!(line.attr("rank").and_then(Value::as_i64), Some(-3));
        assert_eq!(line.attr("missing"), None);
        let rank = roads.key_index("rank").expect("key");
        assert_eq!(line.attr_at(rank), Some(&Value::Sint(-3)));
        assert_eq!(roads.key_index("nope"), None);
        let attrs: Vec<_> = line.attrs().map(|(k, _)| k).collect();
        assert_eq!(attrs, ["class", "name", "rank"]);

        let poly = roads.feature(1).expect("polygon");
        assert_eq!((poly.id(), poly.geom_type()), (None, GeomType::Polygon));
        let roles: Vec<_> = poly.parts().map(|p| (p.role, p.points.len())).collect();
        assert_eq!(roles, [(PartRole::Exterior, 4), (PartRole::Interior, 4)]);
        assert_eq!(poly.attr("class").and_then(Value::as_f64), Some(2.5));
        assert_eq!(poly.attr("rank").and_then(Value::as_f64), Some(1.0));

        let multi = roads.feature(2).expect("points");
        let parts: Vec<_> = multi.parts().map(|p| (p.role, pts(p.points))).collect();
        assert_eq!(parts, [(PartRole::Points, vec![(5, 5), (-20, 4200)])]);
        assert_eq!(multi.attr("rank").and_then(Value::as_i64), Some(-7));
        assert_eq!(multi.attr("name").and_then(Value::as_bool), Some(true));
        assert_eq!(roads.vertex_count(), 5 + 8 + 2);

        let places = t.layer("places").expect("places");
        assert_eq!(places.extent(), 512);
        let p = places.feature(0).expect("place");
        assert_eq!(p.attr("name").and_then(Value::as_str), Some("Testville"));
        assert!(roads.feature(3).is_none());
        assert!(t.layer("water").is_none());
    }

    #[test]
    fn specification_examples() {
        // The geometry examples of the MVT 2.1 specification, section 4.3.5.
        let decode_one = |geom_type: u32, ints: &[u32]| {
            let mut l = LayerBuilder::new("x");
            l.features.push(feature(None, geom_type, &[], ints));
            let t = decode(&one_layer(&l)).expect("valid");
            let layer = &t.layers()[0];
            layer
                .feature(0)
                .expect("feature")
                .parts()
                .map(|p| (p.role, pts(p.points)))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            decode_one(1, &[9, 50, 34]),
            [(PartRole::Points, vec![(25, 17)])]
        );
        assert_eq!(
            decode_one(1, &[17, 10, 14, 3, 9]),
            [(PartRole::Points, vec![(5, 7), (3, 2)])]
        );
        assert_eq!(
            decode_one(2, &[9, 4, 4, 18, 0, 16, 16, 0]),
            [(PartRole::Line, vec![(2, 2), (2, 10), (10, 10)])]
        );
        assert_eq!(
            decode_one(2, &[9, 4, 4, 18, 0, 16, 16, 0, 9, 17, 17, 10, 4, 8]),
            [
                (PartRole::Line, vec![(2, 2), (2, 10), (10, 10)]),
                (PartRole::Line, vec![(1, 1), (3, 5)]),
            ]
        );
        assert_eq!(
            decode_one(3, &[9, 6, 12, 18, 10, 12, 24, 44, 15]),
            [(PartRole::Exterior, vec![(3, 6), (8, 12), (20, 34)])]
        );
        let multi = [
            9, 0, 0, 26, 20, 0, 0, 20, 19, 0, 15, 9, 22, 2, 26, 18, 0, 0, 18, 17, 0, 15, 9, 4, 13,
            26, 0, 8, 8, 0, 0, 7, 15,
        ];
        assert_eq!(
            decode_one(3, &multi),
            [
                (PartRole::Exterior, vec![(0, 0), (10, 0), (10, 10), (0, 10)]),
                (
                    PartRole::Exterior,
                    vec![(11, 11), (20, 11), (20, 20), (11, 20)]
                ),
                (
                    PartRole::Interior,
                    vec![(13, 13), (13, 17), (17, 17), (17, 13)]
                ),
            ]
        );
    }

    #[test]
    fn empty_input_is_an_empty_tile() {
        let t = decode(&[]).expect("empty is valid");
        assert!(t.layers().is_empty());
    }

    #[test]
    fn unknown_fields_are_skipped_at_every_level() {
        let junk = |out: &mut Vec<u8>| {
            varint_field(out, 99, 12345);
            key(out, 98, 1);
            out.extend_from_slice(&[1; 8]);
            len_field(out, 97, b"ignored");
            key(out, 96, 5);
            out.extend_from_slice(&[2; 4]);
        };
        let mut v = Vec::new();
        junk(&mut v);
        v.extend(value(&TestValue::Str("forest")));
        let mut f = Vec::new();
        junk(&mut f);
        f.extend(feature(
            None,
            3,
            &[0, 0],
            &polygon(&[&[(0, 0), (9, 0), (9, 9)]]),
        ));
        let mut l = LayerBuilder::new("land");
        l.keys.push("class".into());
        l.values.push(v);
        l.features.push(f);
        let mut layer = Vec::new();
        junk(&mut layer);
        layer.extend(l.encode());
        let mut t = Vec::new();
        junk(&mut t);
        len_field(&mut t, 3, &layer);
        junk(&mut t);
        let t = decode(&t).expect("valid");
        let f = t.layer("land").and_then(|l| l.feature(0)).expect("feature");
        assert_eq!(f.attr("class").and_then(Value::as_str), Some("forest"));
        assert_eq!(f.parts().len(), 1);
    }

    #[test]
    fn values_of_every_type() {
        let mut l = LayerBuilder::new("v");
        let all = [
            TestValue::Str("s"),
            TestValue::Float(1.5),
            TestValue::Double(-2.25),
            TestValue::Int(-5),
            TestValue::Uint(u64::MAX),
            TestValue::Sint(i64::MIN),
            TestValue::Bool(false),
            TestValue::Double(f64::NAN),
            TestValue::Double(3e19),
        ];
        for (i, v) in all.iter().enumerate() {
            l.keys.push(format!("k{i}"));
            l.values.push(value(v));
        }
        let tags: Vec<u32> = (0..all.len() as u32).flat_map(|i| [i, i]).collect();
        l.features.push(feature(None, 1, &tags, &points(&[(1, 1)])));
        let t = decode(&one_layer(&l)).expect("valid");
        let f = t.layers()[0].feature(0).expect("feature");
        let get = |k: &str| f.attr(k).expect("attribute");
        assert_eq!(get("k0").as_str(), Some("s"));
        assert_eq!(get("k1").as_f64(), Some(1.5));
        assert_eq!(get("k1").as_i64(), None);
        assert_eq!(get("k2"), &Value::Double(-2.25));
        assert_eq!(get("k3").as_i64(), Some(-5));
        assert_eq!(get("k4"), &Value::Uint(u64::MAX));
        assert_eq!(get("k4").as_i64(), None);
        assert_eq!(get("k5").as_i64(), Some(i64::MIN));
        assert_eq!(get("k6").as_bool(), Some(false));
        assert_eq!(get("k6").as_f64(), None);
        assert_eq!(get("k7").as_f64(), None);
        assert_eq!(get("k7").as_i64(), None);
        assert_eq!(get("k8").as_i64(), None);
        assert_eq!(Value::Double(-4.0).as_i64(), Some(-4));
        assert_eq!(Value::Float(f32::INFINITY).as_f64(), None);
    }

    #[test]
    fn unsupported_versions_and_geometry_types_are_skipped() {
        let mut v1 = LayerBuilder::new("old");
        v1.version = Some(1);
        v1.features.push(feature(None, 1, &[], &points(&[(1, 1)])));
        let mut v3 = LayerBuilder::new("future");
        v3.version = Some(3);
        let mut none = LayerBuilder::new("noversion");
        none.version = None;
        let mut ok = LayerBuilder::new("ok");
        ok.keys.push("k".into());
        ok.values.push(value(&TestValue::Bool(true)));
        ok.features.push(feature(None, 0, &[0, 0], &[9, 2, 2]));
        ok.features.push(feature(None, 4, &[0, 0, 0], &[1, 2, 3]));
        ok.features
            .push(feature(None, 1, &[0, 0], &points(&[(1, 1)])));
        // The skipped "old" layer may share a name with a kept one.
        let mut old_ok = LayerBuilder::new("ok");
        old_ok.version = Some(1);
        let t = decode(&tile(&[
            v1.encode(),
            v3.encode(),
            none.encode(),
            ok.encode(),
            old_ok.encode(),
        ]))
        .expect("valid");
        assert_eq!(t.skipped_layers(), 4);
        assert_eq!(t.layers().len(), 1);
        let layer = t.layer("ok").expect("kept");
        assert_eq!((layer.len(), layer.skipped_features()), (1, 2));
        assert_eq!(
            layer.feature(0).and_then(|f| f.attr("k").cloned()),
            Some(Value::Bool(true))
        );
        // A skipped layer is still checked: a bad tag there is an error.
        let mut bad_old = LayerBuilder::new("old");
        bad_old.version = Some(1);
        bad_old
            .features
            .push(feature(None, 1, &[0, 0], &points(&[(1, 1)])));
        assert_eq!(
            decode(&one_layer(&bad_old)).err(),
            Some(MvtError::TagOutOfRange)
        );
    }

    #[test]
    fn structural_errors() {
        use MvtError::*;
        let check = |l: &LayerBuilder, want: MvtError| {
            assert_eq!(decode(&one_layer(l)).err(), Some(want), "{l:?}");
        };
        let base = || {
            let mut l = LayerBuilder::new("l");
            l.keys.push("k".into());
            l.values.push(value(&TestValue::Int(1)));
            l
        };
        let with_feature = |tags: &[u32], geom_type: u32, geom: &[u32]| {
            let mut l = base();
            l.features.push(feature(None, geom_type, tags, geom));
            l
        };
        let p = points(&[(1, 1)]);
        check(&with_feature(&[0], 1, &p), OddTags);
        check(&with_feature(&[1, 0], 1, &p), TagOutOfRange);
        check(&with_feature(&[0, 1], 1, &p), TagOutOfRange);
        check(&with_feature(&[0, 0, 0, u32::MAX], 1, &p), TagOutOfRange);
        check(&LayerBuilder::new(""), MissingLayerName);
        let mut l = base();
        l.name = None;
        check(&l, MissingLayerName);
        let mut l = base();
        l.values.push(Vec::new());
        check(&l, BadValue);
        let mut l = base();
        let mut two = value(&TestValue::Int(1));
        two.extend(value(&TestValue::Bool(true)));
        l.values.push(two);
        check(&l, BadValue);
        let mut l = base();
        let mut only_junk = Vec::new();
        varint_field(&mut only_junk, 8, 1);
        l.values.push(only_junk);
        check(&l, BadValue);
        let mut l = base();
        let mut bad = Vec::new();
        len_field(&mut bad, 1, &[0xff, 0xfe]);
        l.values.push(bad);
        check(&l, BadUtf8);
        let mut l = base();
        let mut wrong = Vec::new();
        varint_field(&mut wrong, 2, 1); // float as a varint
        l.values.push(wrong);
        check(&l, WrongWireType { field: 2 });
        // Duplicate names.
        let a = base().encode();
        assert_eq!(
            decode(&tile(&[a.clone(), a])).err(),
            Some(DuplicateLayerName)
        );
        // Known fields with the wrong wire type, at each level.
        let mut t = Vec::new();
        varint_field(&mut t, 3, 1);
        assert_eq!(decode(&t).err(), Some(WrongWireType { field: 3 }));
        let mut raw = Vec::new();
        len_field(&mut raw, 1, b"l");
        varint_field(&mut raw, 1, 5);
        let mut t = Vec::new();
        len_field(&mut t, 3, &raw);
        assert_eq!(decode(&t).err(), Some(WrongWireType { field: 1 }));
        let mut raw = Vec::new();
        len_field(&mut raw, 1, b"l");
        len_field(&mut raw, 5, b"x");
        let mut t = Vec::new();
        len_field(&mut t, 3, &raw);
        assert_eq!(decode(&t).err(), Some(WrongWireType { field: 5 }));
        let mut f = Vec::new();
        len_field(&mut f, 3, b"x");
        let mut l = base();
        l.features.push(f);
        check(&l, WrongWireType { field: 3 });
        let mut f = Vec::new();
        key(&mut f, 4, 5);
        f.extend_from_slice(&[0; 4]);
        let mut l = base();
        l.features.push(f);
        check(&l, WrongWireType { field: 4 });
        // Wire types that do not exist or are groups, and field 0.
        for wire in [3u8, 4, 6, 7] {
            let mut t = Vec::new();
            key(&mut t, 5, wire);
            assert_eq!(decode(&t).err(), Some(BadWireType(wire)));
        }
        assert_eq!(decode(&[0x02, 0x00]).err(), Some(BadFieldNumber));
        let mut t = Vec::new();
        put_varint(&mut t, (1u64 << 32) | 2);
        assert_eq!(decode(&t).err(), Some(BadFieldNumber));
    }

    #[test]
    fn geometry_errors() {
        use MvtError::*;
        let check = |geom_type: u32, ints: &[u32]| {
            let mut l = LayerBuilder::new("g");
            l.features.push(feature(None, geom_type, &[], ints));
            decode(&one_layer(&l)).err()
        };
        let bad = |g, ints: &[u32]| {
            assert!(
                matches!(check(g, ints), Some(BadGeometry(_))),
                "{g} {ints:?}"
            );
        };
        bad(1, &[]);
        bad(1, &[command(MOVE_TO, 0)]);
        bad(1, &[command(MOVE_TO, 2), 2, 2]);
        bad(1, &[command(LINE_TO, 1), 2, 2]);
        bad(1, &[command(MOVE_TO, 1), 2, 2, command(LINE_TO, 1), 2, 2]);
        bad(1, &[command(MOVE_TO, 1), 2, 2, 0]);
        bad(1, &[command(3, 1), 2, 2]);
        // A huge count is refused without looping over it.
        bad(1, &[command(MOVE_TO, (1 << 29) - 1), 2, 2]);
        bad(2, &[command(MOVE_TO, 1), 2, 2]);
        bad(2, &[command(MOVE_TO, 2), 2, 2, 4, 4]);
        bad(2, &[command(MOVE_TO, 1), 2, 2, command(LINE_TO, 0)]);
        bad(2, &[command(LINE_TO, 1), 2, 2]);
        bad(2, &[command(MOVE_TO, 1), 2, 2, command(LINE_TO, 1), 2]);
        bad(2, &[command(MOVE_TO, 1), 2, 2, command(CLOSE_PATH, 1)]);
        let ring = [command(MOVE_TO, 1), 0, 0, command(LINE_TO, 2), 20, 0, 0, 20];
        bad(3, &ring);
        let mut two_close = ring.to_vec();
        two_close.push(command(CLOSE_PATH, 2));
        bad(3, &two_close);
        bad(
            3,
            &[
                command(MOVE_TO, 1),
                0,
                0,
                command(LINE_TO, 1),
                20,
                0,
                command(CLOSE_PATH, 1),
            ],
        );
        let mut extra = ring.to_vec();
        extra.extend([command(CLOSE_PATH, 1), command(LINE_TO, 1), 2, 2]);
        bad(3, &extra);
        // Coordinates: the extremes of i32 are fine, one more is not.
        let edge = [command(MOVE_TO, 1), zigzag(i32::MAX), zigzag(i32::MIN)];
        assert_eq!(check(1, &edge), None);
        let mut over = edge.to_vec();
        over[0] = command(MOVE_TO, 2);
        over.extend([zigzag(1), 0]);
        assert_eq!(check(1, &over), Some(CoordinateOutOfRange));
        over[3..].copy_from_slice(&[0, zigzag(-1)]);
        assert_eq!(check(1, &over), Some(CoordinateOutOfRange));
        // A zero-area ring is dropped, not an error.
        let flat = [
            command(MOVE_TO, 1),
            0,
            0,
            command(LINE_TO, 2),
            20,
            0,
            20,
            0,
            command(CLOSE_PATH, 1),
        ];
        let mut l = LayerBuilder::new("g");
        l.features.push(feature(None, 3, &[], &flat));
        let t = decode(&one_layer(&l)).expect("valid");
        assert_eq!(t.layers()[0].feature(0).expect("feature").parts().len(), 0);
        assert_eq!(t.layers()[0].vertex_count(), 0);
        // Unpacked geometry and tags are read like packed ones.
        let mut f = Vec::new();
        varint_field(&mut f, 3, 1);
        for int in [command(MOVE_TO, 1), zigzag(3), zigzag(4)] {
            varint_field(&mut f, 4, u64::from(int));
        }
        varint_field(&mut f, 2, 0);
        varint_field(&mut f, 2, 0);
        let mut l = LayerBuilder::new("g");
        l.keys.push("k".into());
        l.values.push(value(&TestValue::Int(1)));
        l.features.push(f);
        let t = decode(&one_layer(&l)).expect("valid");
        let f = t.layers()[0].feature(0).expect("feature");
        let parts: Vec<_> = f.parts().map(|p| pts(p.points)).collect();
        assert_eq!(parts, [vec![(3, 4)]]);
        assert_eq!(f.attr("k"), Some(&Value::Int(1)));
    }

    #[test]
    fn varint_edge_cases() {
        use MvtError::*;
        // The longest valid varint: u64::MAX in 10 bytes, as a feature id.
        let mut f = Vec::new();
        varint_field(&mut f, 1, u64::MAX);
        varint_field(&mut f, 3, 1);
        len_field(&mut f, 4, &packed(&points(&[(1, 1)])));
        let mut l = LayerBuilder::new("v");
        l.features.push(f.clone());
        let t = decode(&one_layer(&l)).expect("valid");
        assert_eq!(
            t.layers()[0].feature(0).and_then(|f| f.id()),
            Some(u64::MAX)
        );
        // An overlong but valid encoding: extent 4096 in 5 bytes.
        let mut raw = Vec::new();
        len_field(&mut raw, 1, b"v");
        key(&mut raw, 5, 0);
        raw.extend_from_slice(&[0x80, 0xa0, 0x80, 0x80, 0x00]);
        varint_field(&mut raw, 15, 2);
        let mut t = Vec::new();
        len_field(&mut t, 3, &raw);
        assert_eq!(decode(&t).expect("valid").layers()[0].extent(), 4096);
        // 11 bytes, and a 10th byte holding more than the 64th bit.
        let mut t = Vec::new();
        key(&mut t, 9, 0);
        t.extend_from_slice(&[0xff; 10]);
        t.push(0x01);
        assert_eq!(decode(&t).err(), Some(BadVarint));
        let mut t = Vec::new();
        key(&mut t, 9, 0);
        t.extend_from_slice(&[0xff; 9]);
        t.push(0x02);
        assert_eq!(decode(&t).err(), Some(BadVarint));
        let mut t = Vec::new();
        key(&mut t, 9, 0);
        t.extend_from_slice(&[0x80; 10]);
        assert_eq!(decode(&t).err(), Some(BadVarint));
        // A varint that never ends.
        assert_eq!(decode(&[0x08, 0x80, 0x80]).err(), Some(Truncated));
        // A geometry integer above u32.
        let mut f = Vec::new();
        varint_field(&mut f, 3, 1);
        varint_field(&mut f, 4, 1u64 << 32);
        let mut l = LayerBuilder::new("v");
        l.features.push(f);
        assert_eq!(decode(&one_layer(&l)).err(), Some(NumberTooLarge));
        // A length beyond the end, and one beyond usize on any platform.
        assert_eq!(decode(&[0x1a, 0x05, 0x00]).err(), Some(Truncated));
        let mut t = vec![0x1a];
        put_varint(&mut t, u64::MAX);
        assert_eq!(decode(&t).err(), Some(Truncated));
        // Zigzag round trips at the extremes.
        for v in [0, 1, -1, 2, -2, i32::MAX, i32::MIN] {
            assert_eq!(zigzag32(zigzag(v)), v);
        }
        for v in [0, -1, i64::MAX, i64::MIN] {
            assert_eq!(zigzag64(zigzag_64(v)), v);
        }
        assert_eq!(zigzag32(u32::MAX), i32::MIN);
        assert_eq!(zigzag64(u64::MAX), i64::MIN);
    }

    #[test]
    fn every_truncation_is_an_error() {
        // One layer: only the empty prefix and the whole tile are valid.
        let mut l = LayerBuilder::new("roads");
        l.keys.push("class".into());
        l.values.push(value(&TestValue::Str("primary")));
        l.values.push(value(&TestValue::Double(1.0)));
        l.features.push(feature(
            Some(7),
            2,
            &[0, 0],
            &lines(&[&[(1, 2), (300, 400)]]),
        ));
        l.features.push(feature(
            None,
            3,
            &[0, 1],
            &polygon(&[&[(0, 0), (50, 0), (0, 50)]]),
        ));
        let t = one_layer(&l);
        assert!(decode(&t).is_ok());
        for cut in 1..t.len() {
            assert!(decode(&t[..cut]).is_err(), "prefix of {cut} bytes");
        }
        // The sample tile: prefixes never panic; Ok only at layer ends.
        let sample = sample_tile();
        let first_layer_end = {
            let mut r = Reader::new(&sample);
            r.key().expect("key");
            r.bytes().expect("layer");
            r.pos
        };
        for cut in 0..sample.len() {
            let ok = decode(&sample[..cut]).is_ok();
            assert_eq!(
                ok,
                cut == 0 || cut == first_layer_end,
                "prefix of {cut} bytes"
            );
        }
    }

    #[test]
    fn bounds_hit_exactly_and_exceeded_by_one() {
        use MvtError::*;
        // Layers.
        let layers = |n: usize| {
            let all: Vec<_> = (0..n)
                .map(|i| LayerBuilder::new(&format!("l{i}")).encode())
                .collect();
            decode(&tile(&all))
        };
        assert_eq!(
            layers(MAX_LAYERS).expect("at the bound").layers().len(),
            MAX_LAYERS
        );
        assert_eq!(layers(MAX_LAYERS + 1).err(), Some(TooManyLayers));
        // Keys and values, counted over the whole tile.
        let keys = |n: usize| {
            let mut a = LayerBuilder::new("a");
            let mut b = LayerBuilder::new("b");
            a.keys = vec!["k".into(); n / 2];
            b.keys = vec!["k".into(); n - n / 2];
            decode(&tile(&[a.encode(), b.encode()]))
        };
        assert!(keys(MAX_KEYS).is_ok());
        assert_eq!(keys(MAX_KEYS + 1).err(), Some(TooManyKeys));
        let values = |n: usize| {
            let mut a = LayerBuilder::new("a");
            a.values = vec![value(&TestValue::Bool(true)); n];
            decode(&one_layer(&a))
        };
        assert!(values(MAX_VALUES).is_ok());
        assert_eq!(values(MAX_VALUES + 1).err(), Some(TooManyValues));
        // Features (empty ones: unknown type, skipped but counted).
        let features = |n: usize| {
            let mut a = LayerBuilder::new("a");
            a.features = vec![Vec::new(); n];
            decode(&one_layer(&a))
        };
        let t = features(MAX_FEATURES).expect("at the bound");
        assert_eq!(t.layers()[0].skipped_features(), MAX_FEATURES);
        assert_eq!(features(MAX_FEATURES + 1).err(), Some(TooManyFeatures));
        // Geometry integers: a multipoint of 1 + 2n integers, plus a
        // skipped feature holding the rest.
        let geometry = |total: usize| {
            let n = 999_999;
            let mut big = vec![command(MOVE_TO, n as u32)];
            big.resize(1 + 2 * n, 0);
            let mut a = LayerBuilder::new("a");
            a.features.push(feature(None, 1, &[], &big));
            a.features
                .push(feature(None, 0, &[], &vec![0; total - big.len()]));
            decode(&one_layer(&a))
        };
        let t = geometry(MAX_GEOMETRY_INTEGERS).expect("at the bound");
        assert_eq!(t.layers()[0].vertex_count(), 999_999);
        assert_eq!(
            geometry(MAX_GEOMETRY_INTEGERS + 1).err(),
            Some(TooManyGeometryIntegers)
        );
        // Tag integers.
        let tags = |n: usize| {
            let mut a = LayerBuilder::new("a");
            a.keys.push("k".into());
            a.values.push(value(&TestValue::Int(0)));
            a.features
                .push(feature(None, 1, &vec![0; n], &points(&[(0, 0)])));
            decode(&one_layer(&a))
        };
        assert!(tags(MAX_TAG_INTEGERS).is_ok());
        assert_eq!(tags(MAX_TAG_INTEGERS + 1).err(), Some(TooManyTagIntegers));
        // Strings: layer names, keys and string values.
        let name = "n".repeat(MAX_STRING_BYTES);
        assert!(decode(&one_layer(&LayerBuilder::new(&name))).is_ok());
        let long = "n".repeat(MAX_STRING_BYTES + 1);
        assert_eq!(
            decode(&one_layer(&LayerBuilder::new(&long))).err(),
            Some(StringTooLong)
        );
        let mut a = LayerBuilder::new("a");
        a.keys.push(long.clone());
        assert_eq!(decode(&one_layer(&a)).err(), Some(StringTooLong));
        let mut a = LayerBuilder::new("a");
        a.values.push(value(&TestValue::Str(&long)));
        assert_eq!(decode(&one_layer(&a)).err(), Some(StringTooLong));
        // Extent.
        let extent = |e: u64| {
            let mut raw = Vec::new();
            len_field(&mut raw, 1, b"a");
            varint_field(&mut raw, 5, e);
            varint_field(&mut raw, 15, 2);
            let mut t = Vec::new();
            len_field(&mut t, 3, &raw);
            decode(&t).map(|t| t.layers()[0].extent())
        };
        assert_eq!(extent(256), Ok(256));
        assert_eq!(extent(65_536), Ok(65_536));
        for bad in [0, 255, 65_537, u64::from(u32::MAX) + 256, u64::MAX] {
            assert_eq!(extent(bad), Err(BadExtent(bad)));
        }
        // Tile size: an unknown field filling exactly the limit.
        let filler = |total: usize| {
            let mut t = Vec::new();
            key(&mut t, 15, 2);
            let len = total - 1 - 4; // the length takes 4 bytes here
            put_varint(&mut t, len as u64);
            assert_eq!(t.len(), 5);
            t.resize(total, 0);
            t
        };
        assert!(
            decode(&filler(MAX_TILE_BYTES))
                .expect("at the bound")
                .layers()
                .is_empty()
        );
        assert_eq!(decode(&filler(MAX_TILE_BYTES + 1)).err(), Some(TooLarge));
    }

    /// xorshift64: small, deterministic, good enough to shake a parser.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    /// Whatever decodes must be internally consistent.
    fn check_invariants(t: &Tile) {
        assert!(t.layers().len() <= MAX_LAYERS);
        for l in t.layers() {
            assert!((MIN_EXTENT..=MAX_EXTENT).contains(&l.extent()));
            assert!(!l.name().is_empty());
            for f in l.features() {
                for p in f.parts() {
                    assert!(!p.points.is_empty());
                    if matches!(p.role, PartRole::Exterior | PartRole::Interior) {
                        assert!(p.points.len() >= 3);
                    }
                }
                for (k, _) in f.attrs() {
                    assert!(f.attr(k).is_some());
                }
            }
        }
    }

    #[test]
    fn random_and_mutated_tiles_never_panic() {
        let mut rng = Rng(0x0123_4567_89ab_cdef);
        let mut decoded = 0;
        // Random bytes.
        for _ in 0..2_000 {
            let len = rng.below(300);
            let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            if let Ok(t) = decode(&bytes) {
                check_invariants(&t);
            }
        }
        // Mutations of a valid tile: flips, random bytes, insertions,
        // deletions, duplicated slices, truncation.
        let sample = sample_tile();
        for _ in 0..4_000 {
            let mut t = sample.clone();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(t.len());
                match rng.below(6) {
                    0 => t[at] ^= 1 << rng.below(8),
                    1 => t[at] = rng.next() as u8,
                    2 => t.insert(at, rng.next() as u8),
                    3 => {
                        t.remove(at);
                    }
                    4 => {
                        let end = (at + rng.below(16)).min(t.len());
                        let slice = t[at..end].to_vec();
                        let to = rng.below(t.len());
                        t.splice(to..to, slice);
                    }
                    _ => t.truncate(at),
                }
                if t.is_empty() {
                    break;
                }
            }
            if let Ok(tile) = decode(&t) {
                decoded += 1;
                check_invariants(&tile);
            }
        }
        // Structurally plausible random tiles: valid framing around random
        // tags, geometry commands and values, so the deep paths run.
        for _ in 0..2_000 {
            let mut layers = Vec::new();
            for li in 0..1 + rng.below(3) {
                let mut l = LayerBuilder::new(&format!("l{li}"));
                if rng.below(4) == 0 {
                    l.extent = Some(rng.below(70_000) as u32);
                }
                for _ in 0..rng.below(4) {
                    l.keys.push("k".into());
                }
                for _ in 0..rng.below(4) {
                    l.values.push(value(&TestValue::Sint(rng.next() as i64)));
                }
                for _ in 0..rng.below(5) {
                    let tags: Vec<u32> = (0..rng.below(5)).map(|_| rng.below(5) as u32).collect();
                    let mut geom = Vec::new();
                    for _ in 0..rng.below(6) {
                        let id = [MOVE_TO, LINE_TO, CLOSE_PATH, 3][rng.below(4)];
                        geom.push(command(id, rng.below(4) as u32));
                        for _ in 0..rng.below(6) {
                            geom.push(if rng.below(8) == 0 {
                                rng.next() as u32
                            } else {
                                rng.below(40) as u32
                            });
                        }
                    }
                    l.features
                        .push(feature(None, rng.below(5) as u32, &tags, &geom));
                }
                layers.push(l.encode());
            }
            if let Ok(t) = decode(&tile(&layers)) {
                decoded += 1;
                check_invariants(&t);
            }
        }
        // The mutations and structured tiles must reach Ok sometimes, or
        // they would test only the first error check.
        assert!(decoded > 100, "{decoded}");
    }
}
