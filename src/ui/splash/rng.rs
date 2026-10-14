//! A small seeded RNG (xorshift64*) for splash styles (pure).

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

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::num::NonZeroU32;

    use super::{Rng, splitmix64};

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        assert!((0..10).all(|_| a.next_u64() == b.next_u64()));
    }

    #[test]
    fn zero_seed_is_usable() {
        let mut r = Rng::new(0);
        assert_ne!(r.next_u64(), 0);
        assert_eq!(Rng::new(0), Rng::new(0));
    }

    #[test]
    fn seeding_matches_the_splitmix64_reference() {
        // The first outputs of splitmix64 seeded with 0, from the reference
        // implementation (Vigna); state n+1 is seed n + GOLDEN.
        assert_eq!(splitmix64(0), 0xE220_A839_7B1D_CDAF);
        assert_eq!(Rng::new(0).0, 0xE220_A839_7B1D_CDAF);
        assert_eq!(splitmix64(0x9E37_79B9_7F4A_7C15), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn adjacent_seeds_diverge_immediately() {
        // Unrelated 64-bit states differ in about 32 bits. Seeding with the
        // raw value gave 5 and 6 states 0xa000004 and 0xc000007 after one
        // step: four differing bits, and nearly all bits zero.
        let (a, b) = (Rng::new(5), Rng::new(6));
        let differing = (a.0 ^ b.0).count_ones();
        assert!((16..=48).contains(&differing), "{a:?} vs {b:?}");
        for seed in 1..=8 {
            let ones = Rng::new(seed).0.count_ones();
            assert!((16..=48).contains(&ones), "seed {seed}: {ones} bits set");
        }
        let (mut a, mut b) = (a, b);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn below_stays_in_bound() {
        let mut r = Rng::new(7);
        let bound = NonZeroU32::new(5).unwrap();
        let mut seen = [false; 5];
        for _ in 0..200 {
            let v = r.below(bound);
            assert_matches!(v, 0..=4);
            seen[v as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(r.below(NonZeroU32::MIN), 0);
    }

    #[test]
    fn unit_is_in_half_open_range() {
        let mut r = Rng::new(9);
        assert!((0..1000).all(|_| (0.0..1.0).contains(&r.unit())));
    }
}
