//! The engine's random number functions, which RCBot calls as `RANDOM_LONG` / `RANDOM_FLOAT`.

/// `RANDOM_LONG` and `RANDOM_FLOAT`. The host implements this with the engine's functions (so
/// bots share the engine's sequence, as upstream does); tests use [`XorShiftRandom`].
pub trait Random {
    /// `RANDOM_LONG(lo, hi)`: an integer in `lo..=hi`.
    fn random_long(&mut self, lo: i32, hi: i32) -> i32;
    /// `RANDOM_FLOAT(lo, hi)`: a float in `lo..hi`.
    fn random_float(&mut self, lo: f32, hi: f32) -> f32;
}

/// A small seeded generator for tests and tools. Not the engine's sequence.
#[derive(Clone, Debug)]
pub struct XorShiftRandom(u64);

impl XorShiftRandom {
    /// A generator from a seed (0 is replaced by a fixed non-zero seed).
    pub fn new(seed: u64) -> XorShiftRandom {
        XorShiftRandom(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

impl Random for XorShiftRandom {
    fn random_long(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        let span = (i64::from(hi) - i64::from(lo) + 1) as u64;
        (i64::from(lo) + (self.next() % span) as i64) as i32
    }

    fn random_float(&mut self, lo: f32, hi: f32) -> f32 {
        let unit = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
        lo + (hi - lo) * unit
    }
}
