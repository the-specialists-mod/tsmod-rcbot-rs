//! The `FILE*` reads and writes RCBot's learning classes do, over bytes.
//!
//! Upstream's loaders `fread` straight into their fields and check `feof` between items, and that
//! behaviour decides what a damaged or mismatched file loads as (a header mismatch consumes the
//! 32-byte header and nothing else, so the next item reads from the wrong place). [`Reader`] keeps
//! exactly that: a short read consumes what is left, copies the bytes it got, and sets the
//! end-of-file flag, which a read that ends exactly at the end does not.

/// A `FILE*` opened for reading on a byte slice.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    eof: bool,
}

impl<'a> Reader<'a> {
    /// A reader at the start of `bytes`.
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0, eof: false }
    }

    /// `feof`: set once a read came up short.
    pub fn eof(&self) -> bool {
        self.eof
    }

    /// Bytes not read yet.
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// `fread(buf, 1, buf.len())`: copies what is there into the front of `buf` (the rest of `buf`
    /// keeps its contents, as with `fread`) and returns how many bytes were copied.
    pub fn read_into(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.remaining());
        buf[..n].copy_from_slice(&self.bytes[self.pos..self.pos + n]);
        self.pos += n;
        if n < buf.len() {
            self.eof = true;
        }
        n
    }

    /// A read that wanted more than is left: consumes the rest and sets end-of-file.
    pub fn exhaust(&mut self) {
        self.pos = self.bytes.len();
        self.eof = true;
    }

    /// `fread(&x, 4, 1)` of a little-endian 32-bit value: `None` on a short read.
    pub fn read_u32(&mut self) -> Option<u32> {
        let mut b = [0u8; 4];
        (self.read_into(&mut b) == 4).then(|| u32::from_le_bytes(b))
    }

    /// `fread(&x, sizeof(int), 1)`.
    pub fn read_i32(&mut self) -> Option<i32> {
        self.read_u32().map(|v| v as i32)
    }

    /// `fread(&x, sizeof(float), 1)`.
    pub fn read_f32(&mut self) -> Option<f32> {
        self.read_u32().map(f32::from_bits)
    }

    /// `fread(&x, sizeof(bool), 1)`; any non-zero byte is true.
    pub fn read_bool(&mut self) -> Option<bool> {
        let mut b = [0u8; 1];
        (self.read_into(&mut b) == 1).then(|| b[0] != 0)
    }

    /// `fread(data, sizeof(float), count)` into a vector resized to `count`: `true` if all of them
    /// were read. A count larger than what is left reads what is there and fails, without
    /// allocating the count first (upstream resizes to whatever a damaged file says).
    pub fn read_f32s(&mut self, count: u32, out: &mut Vec<f32>) -> bool {
        let count = count as usize;
        let whole = (self.remaining() / 4).min(count);
        out.clear();
        out.extend((0..whole).map(|_| self.read_f32().unwrap_or_default()));
        if whole < count {
            // fread consumes the tail of a partial element too.
            self.exhaust();
            return false;
        }
        true
    }
}

/// Appends little-endian values, as `fwrite` of the C++ fields does on x86.
pub(crate) trait WriteLe {
    fn put_u32(&mut self, v: u32);
    fn put_i32(&mut self, v: i32);
    fn put_f32(&mut self, v: f32);
    fn put_f32s(&mut self, v: &[f32]);
}

impl WriteLe for Vec<u8> {
    fn put_u32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_i32(&mut self, v: i32) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_f32(&mut self, v: f32) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_f32s(&mut self, v: &[f32]) {
        for &x in v {
            self.put_f32(x);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eof_only_after_a_short_read() {
        let mut r = Reader::new(&[1, 0, 0, 0, 2, 0]);
        assert_eq!(r.read_u32(), Some(1));
        assert!(!r.eof());
        let mut buf = [9u8; 4];
        assert_eq!(r.read_into(&mut buf), 2);
        assert_eq!(buf, [2, 0, 9, 9]);
        assert!(r.eof());

        let mut exact = Reader::new(&[0, 0, 128, 63]);
        assert_eq!(exact.read_f32(), Some(1.0));
        assert!(!exact.eof());
    }

    #[test]
    fn oversized_counts_fail_without_allocating() {
        let bytes = [0, 0, 128, 63, 0, 0, 0, 64, 7];
        let mut r = Reader::new(&bytes);
        let mut v = Vec::new();
        assert!(!r.read_f32s(u32::MAX, &mut v));
        assert_eq!(v, [1.0, 2.0]);
        assert!(r.eof());
        assert_eq!(r.remaining(), 0);
    }
}
