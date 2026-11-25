//! A small seeded random source, so the same seed writes the same song on
//! every machine and every build.

/// SplitMix64: tiny, fast, and the same everywhere.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    /// A second stream from this seed and `salt`, independent of the first:
    /// each part of a song draws from its own, so changing how one part is
    /// written leaves the others as they were.
    pub fn fork(seed: u64, salt: u64) -> Rng {
        let mut r = Rng::new(seed ^ salt.wrapping_mul(0xD1B5_4A32_D192_ED03));
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0 <= x < n`; 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// `0.0 <= x < 1.0`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Between `lo` and `hi`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// True `p` of the time.
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.below(items.len())])
        }
    }

    /// An index into `weights`, each chosen in proportion to its weight.
    pub fn weighted(&mut self, weights: &[f32]) -> usize {
        let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
        if total <= 0.0 {
            return self.below(weights.len());
        }
        let mut x = self.unit() * total;
        for (i, w) in weights.iter().enumerate() {
            x -= w.max(0.0);
            if x < 0.0 {
                return i;
            }
        }
        weights.len() - 1
    }
}

/// A stable 64-bit hash of some text, for seeds made from names and dates.
pub fn hash(text: &str) -> u64 {
    // FNV-1a.
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01B3);
    }
    h
}
