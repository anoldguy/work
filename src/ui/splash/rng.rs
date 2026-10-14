//! A small seeded RNG (xorshift64*) for splash styles (pure).

use std::num::NonZeroU32;

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
        let _ = seed;
        unimplemented!("W1.9")
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let _ = &mut self.0;
        unimplemented!("W1.9")
    }

    /// A uniform value in `0..bound`.
    pub fn below(&mut self, bound: NonZeroU32) -> u32 {
        let _ = (self, bound);
        unimplemented!("W1.9")
    }

    /// A uniform value in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        let _ = self;
        unimplemented!("W1.9")
    }
}
