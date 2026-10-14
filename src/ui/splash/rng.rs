//! A small seeded RNG (xorshift64*) for splash styles (pure).

use std::num::NonZeroU32;

const FALLBACK_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// xorshift64* state.
///
/// Invariant: the state is never zero (a zero seed is replaced by a fixed
/// non-zero constant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    /// Seeds the generator.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { FALLBACK_SEED } else { seed })
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

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::num::NonZeroU32;

    use super::Rng;

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
    fn known_first_output_for_seed_one() {
        // x=1: x ^= x>>12 -> 1; x ^= x<<25 -> 1|1<<25; x ^= x>>27 -> unchanged.
        let x: u64 = 1 | (1 << 25);
        assert_eq!(
            Rng::new(1).next_u64(),
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        );
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
