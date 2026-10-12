//! RCBot's compressed file container (`rc_compress.{h,cpp}`): a 16-byte header (`"RCZ\0"`,
//! version 1, uncompressed size, compressed size, little-endian `uint32`s) and a zlib stream that
//! miniz writes with `mz_compress2(…, MZ_BEST_COMPRESSION)`. The visibility table (`.rcv`) uses
//! it; learned data will too.

use miniz_oxide::deflate::compress_to_vec_zlib;
use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;

/// `RCZ_MAGIC`.
pub const MAGIC: [u8; 4] = *b"RCZ\0";
/// `RCZ_VERSION`.
pub const VERSION: u32 = 1;
/// `sizeof(RCZHeader)`.
pub const HEADER_SIZE: usize = 16;
/// `MZ_BEST_COMPRESSION`.
const LEVEL: u8 = 9;

/// `RCZ_IsCompressedFile`: at least a header's worth of bytes starting with the magic.
pub fn is_compressed(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER_SIZE && bytes[..4] == MAGIC
}

/// `RCZ_CompressToFileHandle`: `None` for empty data, as upstream refuses it.
pub fn compress(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }
    let z = compress_to_vec_zlib(data, LEVEL);
    let mut out = Vec::with_capacity(HEADER_SIZE + z.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(z.len() as u32).to_le_bytes());
    out.extend_from_slice(&z);
    Some(out)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// `RCZ_DecompressFromFileHandle`: the data if the header is valid, its uncompressed size is
/// `expected_size`, and the stream inflates to exactly that many bytes.
pub fn decompress(bytes: &[u8], expected_size: usize) -> Option<Vec<u8>> {
    if expected_size == 0 || !is_compressed(bytes) || u32_at(bytes, 4) != VERSION || u32_at(bytes, 8) as usize != expected_size {
        return None;
    }
    let len = u32_at(bytes, 12) as usize;
    let z = bytes.get(HEADER_SIZE..HEADER_SIZE.checked_add(len)?)?;
    let out = decompress_to_vec_zlib_with_limit(z, expected_size).ok()?;
    (out.len() == expected_size).then_some(out)
}

/// `RCZ_DecompressFileToMemory`: the data of any valid container, whatever its size.
pub fn decompress_any(bytes: &[u8]) -> Option<Vec<u8>> {
    if !is_compressed(bytes) || u32_at(bytes, 4) != VERSION || u32_at(bytes, 8) == 0 || u32_at(bytes, 12) == 0 {
        return None;
    }
    decompress(bytes, u32_at(bytes, 8) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_validation() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 13) as u8).collect();
        let c = compress(&data).unwrap();
        assert!(is_compressed(&c));
        assert_eq!(decompress(&c, data.len()).unwrap(), data);
        assert_eq!(decompress_any(&c).unwrap(), data);
        assert!(decompress(&c, data.len() - 1).is_none());
        assert!(decompress(&c[..c.len() - 1], data.len()).is_none());
        let mut bad = c.clone();
        bad[4] = 2;
        assert!(decompress(&bad, data.len()).is_none());
        assert!(compress(&[]).is_none());
        assert!(!is_compressed(b"RCZ\0short"));
    }
}
