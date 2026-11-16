//! Test-only Mapbox Vector Tile encoder, so tests can build tiles by hand,
//! including malformed ones (every helper writes exactly what it is given).
//!
//! It is compiled only for this crate's tests and for crates that enable the
//! `test-support` feature, which `resilience-maps` does from its
//! `[dev-dependencies]` only: production code never needs to write tiles,
//! and the pack builder gets its tiles from tippecanoe.

/// Append a base-128 varint.
pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Append a field key with any wire type, valid or not.
pub fn key(out: &mut Vec<u8>, field: u32, wire: u8) {
    put_varint(out, (u64::from(field) << 3) | u64::from(wire));
}

pub fn varint_field(out: &mut Vec<u8>, field: u32, v: u64) {
    key(out, field, 0);
    put_varint(out, v);
}

pub fn len_field(out: &mut Vec<u8>, field: u32, data: &[u8]) {
    key(out, field, 2);
    put_varint(out, data.len() as u64);
    out.extend_from_slice(data);
}

pub fn zigzag(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

pub fn zigzag_64(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// A geometry command integer.
pub fn command(id: u32, count: u32) -> u32 {
    (count << 3) | (id & 7)
}

/// Packed `uint32` bytes.
pub fn packed(ints: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    for &i in ints {
        put_varint(&mut out, u64::from(i));
    }
    out
}

fn delta(cursor: &mut (i32, i32), p: (i32, i32), out: &mut Vec<u32>) {
    out.push(zigzag(p.0.wrapping_sub(cursor.0)));
    out.push(zigzag(p.1.wrapping_sub(cursor.1)));
    *cursor = p;
}

/// Geometry of a (multi)point feature.
pub fn points(pts: &[(i32, i32)]) -> Vec<u32> {
    let mut out = vec![command(1, pts.len() as u32)];
    let mut cursor = (0, 0);
    for &p in pts {
        delta(&mut cursor, p, &mut out);
    }
    out
}

/// Geometry of a (multi)line string.
pub fn lines(parts: &[&[(i32, i32)]]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut cursor = (0, 0);
    for line in parts {
        out.push(command(1, 1));
        delta(&mut cursor, line[0], &mut out);
        out.push(command(2, (line.len() - 1) as u32));
        for &p in &line[1..] {
            delta(&mut cursor, p, &mut out);
        }
    }
    out
}

/// Geometry of a polygon: rings without the closing vertex, wound as the
/// caller gives them.
pub fn polygon(rings: &[&[(i32, i32)]]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut cursor = (0, 0);
    for ring in rings {
        out.push(command(1, 1));
        delta(&mut cursor, ring[0], &mut out);
        out.push(command(2, (ring.len() - 1) as u32));
        for &p in &ring[1..] {
            delta(&mut cursor, p, &mut out);
        }
        out.push(command(7, 1));
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub enum TestValue<'a> {
    Str(&'a str),
    Float(f32),
    Double(f64),
    Int(i64),
    Uint(u64),
    Sint(i64),
    Bool(bool),
}

/// An encoded `Value` message.
pub fn value(v: &TestValue<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    match *v {
        TestValue::Str(s) => len_field(&mut out, 1, s.as_bytes()),
        TestValue::Float(f) => {
            key(&mut out, 2, 5);
            out.extend_from_slice(&f.to_le_bytes());
        }
        TestValue::Double(d) => {
            key(&mut out, 3, 1);
            out.extend_from_slice(&d.to_le_bytes());
        }
        TestValue::Int(i) => varint_field(&mut out, 4, i as u64),
        TestValue::Uint(u) => varint_field(&mut out, 5, u),
        TestValue::Sint(i) => varint_field(&mut out, 6, zigzag_64(i)),
        TestValue::Bool(b) => varint_field(&mut out, 7, u64::from(b)),
    }
    out
}

/// An encoded `Feature` message; empty tags or geometry are left out.
pub fn feature(id: Option<u64>, geom_type: u32, tags: &[u32], geometry: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(id) = id {
        varint_field(&mut out, 1, id);
    }
    if !tags.is_empty() {
        len_field(&mut out, 2, &packed(tags));
    }
    varint_field(&mut out, 3, u64::from(geom_type));
    if !geometry.is_empty() {
        len_field(&mut out, 4, &packed(geometry));
    }
    out
}

/// A `Layer` message; `None` fields are left out.
#[derive(Debug, Clone)]
pub struct LayerBuilder {
    pub name: Option<String>,
    pub version: Option<u32>,
    pub extent: Option<u32>,
    pub keys: Vec<String>,
    /// Encoded `Value` messages (see [`value`]).
    pub values: Vec<Vec<u8>>,
    /// Encoded `Feature` messages (see [`feature`]).
    pub features: Vec<Vec<u8>>,
}

impl LayerBuilder {
    /// A version 2 layer with the default extent.
    pub fn new(name: &str) -> Self {
        LayerBuilder {
            name: Some(name.to_string()),
            version: Some(2),
            extent: None,
            keys: Vec::new(),
            values: Vec::new(),
            features: Vec::new(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(name) = &self.name {
            len_field(&mut out, 1, name.as_bytes());
        }
        for f in &self.features {
            len_field(&mut out, 2, f);
        }
        for k in &self.keys {
            len_field(&mut out, 3, k.as_bytes());
        }
        for v in &self.values {
            len_field(&mut out, 4, v);
        }
        if let Some(extent) = self.extent {
            varint_field(&mut out, 5, u64::from(extent));
        }
        if let Some(version) = self.version {
            varint_field(&mut out, 15, u64::from(version));
        }
        out
    }
}

/// A `Tile` message from encoded layers.
pub fn tile(layers: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for l in layers {
        len_field(&mut out, 3, l);
    }
    out
}
