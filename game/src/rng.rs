//! A tiny deterministic PRNG (SplitMix64). Worldgen must be reproducible from a
//! seed, so we own the generator rather than depending on `rand`'s platform
//! behaviour.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_add(0x9E37_79B9_7F4A_7C15))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        (lo as f64 + (hi - lo) as f64 * self.next_f64()) as i32
    }

    /// A random integer in [0, n) — for picking from a small set.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_f64() * n as f64) as usize
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.next_f64() < p
    }
}
