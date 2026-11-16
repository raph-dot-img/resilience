//! Tile decompression with hard limits (ADR 0016 §7 "MVT and gzip bounds").
//!
//! A tile blob is accepted in three encodings, recognised by its first
//! bytes:
//!
//! - **gzip** (RFC 1952), what tippecanoe and the MBTiles 1.3 specification
//!   use for vector tiles. The header is parsed here (FEXTRA, FNAME,
//!   FCOMMENT and FHCRC are skipped, reserved flags refused); the CRC-32 and
//!   the length in the trailer are checked; bytes after the single member
//!   are refused.
//! - **zlib** (RFC 1950), written by some older tile tools. No preset
//!   dictionary; the Adler-32 is checked.
//! - **uncompressed** MVT, also found in the wild: an empty blob or one
//!   starting with `0x1a` (field 3, a layer, which is the only field of the
//!   `Tile` message). The MVT decoder bounds it like any other input.
//!
//! Anything else (a PNG or JPEG raster tile, random bytes) is refused.
//!
//! A blob larger than [`MAX_COMPRESSED`] is refused before it is looked at.
//! Inflating writes into a buffer that grows by doubling and never beyond
//! [`MAX_DECOMPRESSED`]: when the stream wants more, decompression stops
//! there with [`TileError::TooLargeDecompressed`] (zip bombs cost at most
//! that much memory and the time to produce it).

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
use resilience_core::maps::mvt::{self, MvtError};

/// Largest tile blob accepted, compressed (4 MiB). SQLite's LENGTH limit
/// in `pack.rs` is the same, so a larger blob is not even read.
pub const MAX_COMPRESSED: usize = 4 * 1024 * 1024;
/// Largest tile after decompression (8 MiB), the MVT decoder's input bound.
pub const MAX_DECOMPRESSED: usize = mvt::MAX_TILE_BYTES;

/// Why a tile could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TileError {
    /// The stored blob is over [`MAX_COMPRESSED`].
    TooLargeCompressed,
    /// The blob would decompress to more than [`MAX_DECOMPRESSED`].
    TooLargeDecompressed,
    /// Neither gzip, zlib nor an uncompressed vector tile.
    UnknownEncoding,
    /// A malformed gzip or zlib wrapper (header, checksum, length).
    BadWrapper(&'static str),
    /// The compressed data itself is malformed or cut short.
    BadDeflate,
    /// The decompressed bytes are not a valid vector tile.
    Mvt(MvtError),
}

impl std::fmt::Display for TileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TileError::TooLargeCompressed => write!(
                f,
                "a tile is larger than {} MiB",
                MAX_COMPRESSED / (1024 * 1024)
            ),
            TileError::TooLargeDecompressed => write!(
                f,
                "a tile expands to more than {} MiB (a compression bomb?)",
                MAX_DECOMPRESSED / (1024 * 1024)
            ),
            TileError::UnknownEncoding => {
                write!(f, "a tile is not a vector tile (unknown encoding)")
            }
            TileError::BadWrapper(what) => write!(f, "a tile is damaged ({what})"),
            TileError::BadDeflate => write!(f, "a tile is damaged (compressed data)"),
            TileError::Mvt(e) => write!(f, "a tile is not a valid vector tile ({e})"),
        }
    }
}

impl std::error::Error for TileError {}

/// How a blob is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Gzip,
    Zlib,
    Uncompressed,
}

/// The encoding of `blob`, from its first two bytes.
pub fn detect(blob: &[u8]) -> Option<Encoding> {
    match blob {
        [] | [0x1a, ..] => Some(Encoding::Uncompressed),
        [0x1f, 0x8b, ..] => Some(Encoding::Gzip),
        [cmf, flg, ..] if is_zlib_header(*cmf, *flg) => Some(Encoding::Zlib),
        _ => None,
    }
}

/// RFC 1950: method 8 (deflate), window at most 32 KiB, header check
/// multiple of 31. A preset dictionary (FDICT) is not supported.
fn is_zlib_header(cmf: u8, flg: u8) -> bool {
    cmf & 0x0f == 8
        && cmf >> 4 <= 7
        && (u16::from(cmf) << 8 | u16::from(flg)) % 31 == 0
        && flg & 0x20 == 0
}

/// The tile bytes of `blob` (decompressed if needed), within the limits.
pub fn decompress_tile(blob: &[u8]) -> Result<Vec<u8>, TileError> {
    if blob.len() > MAX_COMPRESSED {
        return Err(TileError::TooLargeCompressed);
    }
    match detect(blob).ok_or(TileError::UnknownEncoding)? {
        Encoding::Uncompressed => Ok(blob.to_vec()),
        Encoding::Zlib => {
            let (out, used) = inflate_limited(blob, true, MAX_DECOMPRESSED)?;
            if used != blob.len() {
                return Err(TileError::BadWrapper("data after the end"));
            }
            Ok(out)
        }
        Encoding::Gzip => gunzip(blob, MAX_DECOMPRESSED),
    }
}

// gzip header flags (RFC 1952 §2.3.1).
const FHCRC: u8 = 0x02;
const FEXTRA: u8 = 0x04;
const FNAME: u8 = 0x08;
const FCOMMENT: u8 = 0x10;
const FRESERVED: u8 = 0xe0;

/// One gzip member, inflated to at most `limit` bytes.
fn gunzip(blob: &[u8], limit: usize) -> Result<Vec<u8>, TileError> {
    let bad = TileError::BadWrapper;
    if blob.len() < 18 {
        return Err(bad("gzip data cut short"));
    }
    if blob[2] != 8 {
        return Err(bad("gzip method is not deflate"));
    }
    let flags = blob[3];
    if flags & FRESERVED != 0 {
        return Err(bad("reserved gzip flags"));
    }
    // ID1 ID2 CM FLG MTIME(4) XFL OS
    let mut pos = 10usize;
    if flags & FEXTRA != 0 {
        let len = blob
            .get(pos..pos + 2)
            .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
            .ok_or(bad("gzip header cut short"))?;
        pos = pos
            .checked_add(2 + len)
            .filter(|&p| p <= blob.len())
            .ok_or(bad("gzip header cut short"))?;
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            let end = blob
                .get(pos..)
                .and_then(|rest| rest.iter().position(|&b| b == 0))
                .ok_or(bad("gzip header cut short"))?;
            pos += end + 1;
        }
    }
    if flags & FHCRC != 0 {
        let stored = blob
            .get(pos..pos + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or(bad("gzip header cut short"))?;
        if stored != (crc32(&blob[..pos]) & 0xffff) as u16 {
            return Err(bad("gzip header checksum"));
        }
        pos += 2;
    }
    let body = blob.get(pos..).ok_or(bad("gzip header cut short"))?;
    let (out, used) = inflate_limited(body, false, limit)?;
    let trailer = &body[used..];
    if trailer.len() < 8 {
        return Err(bad("gzip trailer cut short"));
    }
    if trailer.len() > 8 {
        return Err(bad("data after the end"));
    }
    let crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    let size = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
    if crc != crc32(&out) {
        return Err(bad("gzip checksum"));
    }
    // ISIZE is the length modulo 2^32; the output is far below that.
    if size as usize != out.len() {
        return Err(bad("gzip length"));
    }
    Ok(out)
}

/// Inflate a raw deflate (or, with `zlib`, a zlib) stream into at most
/// `limit` bytes. Returns the output and the number of input bytes used.
fn inflate_limited(input: &[u8], zlib: bool, limit: usize) -> Result<(Vec<u8>, usize), TileError> {
    let mut flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    if zlib {
        flags |=
            inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER | inflate_flags::TINFL_FLAG_COMPUTE_ADLER32;
    }
    let mut out = vec![0u8; input.len().saturating_mul(4).clamp(4096, limit.max(1))];
    let mut state = Box::<DecompressorOxide>::default();
    let (mut in_pos, mut out_pos) = (0usize, 0usize);
    loop {
        let (status, used, written) =
            decompress(&mut state, &input[in_pos..], &mut out, out_pos, flags);
        in_pos += used;
        out_pos += written;
        match status {
            TINFLStatus::Done => {
                out.truncate(out_pos);
                return Ok((out, in_pos));
            }
            TINFLStatus::HasMoreOutput => {
                if out.len() >= limit {
                    return Err(TileError::TooLargeDecompressed);
                }
                let grown = out.len().saturating_mul(2).min(limit);
                out.resize(grown, 0);
            }
            TINFLStatus::Adler32Mismatch => return Err(TileError::BadWrapper("zlib checksum")),
            _ => return Err(TileError::BadDeflate),
        }
        if in_pos > input.len() {
            return Err(TileError::BadDeflate);
        }
    }
}

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// The CRC-32 of gzip (ISO 3309, reflected polynomial 0xedb88320). An
/// integrity check against damage, not a cryptographic hash.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c = CRC_TABLE[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xffff_ffff
}

/// `data` as a minimal gzip member (no optional header fields), as
/// tippecanoe writes it. For tests and for the pack builder's own checks.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(data, 6));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn gzip_round_trip() {
        let data = b"\x1a\x05hello".repeat(100);
        let gz = gzip(&data);
        assert_eq!(detect(&gz), Some(Encoding::Gzip));
        assert_eq!(decompress_tile(&gz).expect("should succeed"), data);
    }

    #[test]
    fn zlib_and_uncompressed_tiles_are_accepted() {
        let data = b"\x1a\x03abc".to_vec();
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        assert_eq!(detect(&z), Some(Encoding::Zlib));
        assert_eq!(decompress_tile(&z).expect("should succeed"), data);
        assert_eq!(decompress_tile(&data).expect("should succeed"), data);
        assert_eq!(
            decompress_tile(&[]).expect("should succeed"),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn other_encodings_are_refused() {
        for blob in [
            &b"\x89PNG\r\n\x1a\n...."[..],
            b"\xff\xd8\xff\xe0JFIF",
            b"RIFF....WEBP",
            b"hello",
        ] {
            assert_eq!(
                decompress_tile(blob),
                Err(TileError::UnknownEncoding),
                "{blob:?}"
            );
        }
    }

    #[test]
    fn optional_gzip_header_fields_are_skipped() {
        let data = b"\x1a\x01x".to_vec();
        let plain = gzip(&data);
        let mut gz = vec![
            0x1f,
            0x8b,
            8,
            FEXTRA | FNAME | FCOMMENT | FHCRC,
            0,
            0,
            0,
            0,
            0,
            3,
        ];
        gz.extend_from_slice(&[3, 0, b'a', b'b', b'c']);
        gz.extend_from_slice(b"tile.pbf\0a comment\0");
        let hcrc = (crc32(&gz) & 0xffff) as u16;
        gz.extend_from_slice(&hcrc.to_le_bytes());
        gz.extend_from_slice(&plain[10..]);
        assert_eq!(decompress_tile(&gz).expect("should succeed"), data);
        // A wrong header checksum is noticed.
        let n = gz.len() - plain.len() + 10;
        gz[n - 1] ^= 1;
        assert_eq!(
            decompress_tile(&gz),
            Err(TileError::BadWrapper("gzip header checksum"))
        );
    }

    #[test]
    fn damaged_gzip_is_refused() {
        let data = b"\x1a\x02ok".repeat(50);
        let gz = gzip(&data);
        let mut crc = gz.clone();
        let n = crc.len();
        crc[n - 6] ^= 0x40;
        assert_eq!(
            decompress_tile(&crc),
            Err(TileError::BadWrapper("gzip checksum"))
        );
        let mut len = gz.clone();
        len[n - 1] ^= 1;
        assert_eq!(
            decompress_tile(&len),
            Err(TileError::BadWrapper("gzip length"))
        );
        let mut extra = gz.clone();
        extra.push(0);
        assert_eq!(
            decompress_tile(&extra),
            Err(TileError::BadWrapper("data after the end"))
        );
        let cut = &gz[..gz.len() - 12];
        assert!(decompress_tile(cut).is_err());
        let mut reserved = gz.clone();
        reserved[3] = 0x20;
        assert_eq!(
            decompress_tile(&reserved),
            Err(TileError::BadWrapper("reserved gzip flags"))
        );
        let mut method = gz.clone();
        method[2] = 7;
        assert!(matches!(
            decompress_tile(&method),
            Err(TileError::BadWrapper(_))
        ));
        assert!(decompress_tile(&gz[..12]).is_err());
    }

    #[test]
    fn a_compression_bomb_stops_at_the_limit() {
        // 64 MiB of zeros compress to about 64 KiB.
        let zeros = vec![0u8; 64 * 1024 * 1024];
        let gz = gzip(&zeros);
        assert!(gz.len() < MAX_COMPRESSED / 16, "{}", gz.len());
        let t = std::time::Instant::now();
        assert_eq!(decompress_tile(&gz), Err(TileError::TooLargeDecompressed));
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        // Exactly at the limit is still accepted.
        let at_limit = gzip(&zeros[..MAX_DECOMPRESSED]);
        assert_eq!(
            decompress_tile(&at_limit).map(|v| v.len()),
            Ok(MAX_DECOMPRESSED)
        );
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&zeros, 1);
        assert_eq!(decompress_tile(&zlib), Err(TileError::TooLargeDecompressed));
    }

    #[test]
    fn a_blob_over_the_compressed_limit_is_refused_unread() {
        let mut big = vec![0x1f, 0x8b];
        big.resize(MAX_COMPRESSED + 1, 0);
        assert_eq!(decompress_tile(&big), Err(TileError::TooLargeCompressed));
    }

    #[test]
    fn random_blobs_never_panic() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let base = gzip(&b"\x1a\x04abcd".repeat(40));
        for i in 0..3000 {
            let mut b = if i % 2 == 0 {
                base.clone()
            } else {
                (0..(next() % 64) as usize).map(|_| next() as u8).collect()
            };
            if !b.is_empty() {
                for _ in 0..(next() % 4 + 1) {
                    let n = b.len();
                    b[(next() as usize) % n] = next() as u8;
                }
            }
            if i % 3 == 0 {
                b.insert(0, 0x1f);
                b.insert(1, 0x8b);
            }
            let _ = decompress_tile(&b);
        }
    }
}
