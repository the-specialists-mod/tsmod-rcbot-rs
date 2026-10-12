//! `CBits` (`bits.{h,cpp}`): a bit array, saved with its own header. The bot's visible-entity set
//! (`CBotVisibles`) and the bit-string GA individual ([`super::ga::BitsGaValues`]) use it.

use super::header::{GenericHeader, LearnType};
use super::io::{Reader, WriteLe};
use crate::Random;

/// `CBits`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bits {
    num_bits: u32,
    bytes: Vec<u8>,
}

impl Bits {
    /// `CBits(iNumBits)`: all clear.
    pub fn new(num_bits: u32) -> Bits {
        Bits { num_bits, bytes: vec![0; num_bits.div_ceil(8) as usize] }
    }

    /// `numBits`.
    pub fn num_bits(&self) -> u32 {
        self.num_bits
    }

    /// `size`: bytes of storage.
    pub fn size(&self) -> usize {
        self.bytes.len()
    }

    /// `getBits`: the storage, bit `i` at `bytes[i / 8] & (1 << i % 8)`.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// `getBits`, writable.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// `setBit`: out-of-range bits are ignored.
    pub fn set(&mut self, bit: u32, on: bool) {
        if bit >= self.num_bits {
            return;
        }
        let byte = &mut self.bytes[(bit / 8) as usize];
        if on {
            *byte |= 1 << (bit % 8);
        } else {
            *byte &= !(1 << (bit % 8));
        }
    }

    /// `getBit`: out-of-range bits are clear.
    pub fn get(&self, bit: u32) -> bool {
        bit < self.num_bits && self.bytes[(bit / 8) as usize] & (1 << (bit % 8)) != 0
    }

    /// `clear`.
    pub fn clear(&mut self) {
        self.bytes.fill(0);
    }

    /// `randomize`: one `RANDOM_FLOAT(0, 1) >= 0.5` per bit, in order.
    pub fn randomize(&mut self, rng: &mut dyn Random) {
        for i in 0..self.num_bits {
            let on = rng.random_float(0.0, 1.0) >= 0.5;
            self.set(i, on);
        }
    }

    /// `save`: header (`LEARNTYPE_BITS`, bit count), `unsigned` bit count, the bytes.
    pub fn save(&self, out: &mut Vec<u8>) {
        GenericHeader::new(LearnType::Bits, self.num_bits as i32).write(out);
        out.put_u32(self.num_bits);
        out.extend_from_slice(&self.bytes);
    }

    /// `load`: the header must name the current bit count; on a mismatch the bits are cleared
    /// and only the header is consumed. Then the stored count replaces the current one.
    pub fn load(&mut self, r: &mut Reader) {
        if !GenericHeader::read(r, &GenericHeader::new(LearnType::Bits, self.num_bits as i32)) {
            self.clear();
            return;
        }
        let Some(n) = r.read_u32() else { return };
        // `setup(n)` allocates n/8 bytes; a damaged count larger than the file is a short read
        // here instead of an allocation of that size.
        let size = n.div_ceil(8) as usize;
        if size > r.remaining() {
            *self = Bits::new(0);
            r.exhaust();
            return;
        }
        *self = Bits::new(n);
        if r.read_into(&mut self.bytes) != size || size == 0 {
            // fread(…, size(), 1) != 1, which includes size() == 0.
            self.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XorShiftRandom;

    #[test]
    fn set_get_and_bounds() {
        let mut b = Bits::new(10);
        assert_eq!(b.size(), 2);
        b.set(9, true);
        b.set(10, true);
        b.set(0, true);
        assert_eq!(b.bytes(), &[1, 2]);
        assert!(b.get(9) && !b.get(10) && !b.get(1));
        b.set(0, false);
        assert_eq!(b.bytes(), &[0, 2]);
    }

    #[test]
    fn save_load_round_trip() {
        let mut b = Bits::new(13);
        b.randomize(&mut XorShiftRandom::new(4));
        let mut out = Vec::new();
        b.save(&mut out);
        assert_eq!(&out[..8], b"t-6-s-13");
        assert_eq!(out.len(), 32 + 4 + 2);
        let mut c = Bits::new(13);
        c.load(&mut Reader::new(&out));
        assert_eq!(c, b);

        // A different current size is a version mismatch.
        let mut d = Bits::new(12);
        d.set(1, true);
        let mut r = Reader::new(&out);
        d.load(&mut r);
        assert_eq!(d.num_bits(), 12);
        assert!(!d.get(1));
        assert_eq!(r.remaining(), 6);
    }
}
