//! A small seeded RNG (xorshift64*) for splash styles.

use std::num::NonZeroU32;

/// The splitmix64 increment (2^64 / φ).
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// xorshift64* state.
///
/// Invariant: the state is never zero (the one seed that mixes to zero is
/// replaced by a fixed non-zero constant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    /// Seeds the generator. The seed is mixed through splitmix64 first, so
    /// small or adjacent seeds give unrelated streams from the first output.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        let state = splitmix64(seed);
        Self(if state == 0 { GOLDEN } else { state })
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A uniform value in `0..bound`.
    pub fn below(&mut self, bound: NonZeroU32) -> u32 {
        let wide = u128::from(self.next_u64()) * u128::from(bound.get());
        // The high 64 bits of a product with a u32 are below `bound`.
        u32::try_from(wide >> 64).unwrap_or(0)
    }

    /// A uniform value in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        // Top 53 bits scaled by 2^-53 give a uniform value in [0, 1).
        #[expect(clippy::cast_precision_loss, reason = "53-bit value is exact in f64")]
        let v = (self.next_u64() >> 11) as f64;
        v * (1.0 / 9_007_199_254_740_992.0)
    }
}

/// The splitmix64 finaliser over `seed + GOLDEN`: a bijection on `u64`
/// that spreads every input bit over the whole output.
const fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(GOLDEN);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
