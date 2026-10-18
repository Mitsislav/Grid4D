//! Helpers shared by this crate's tests.

/// Minimal deterministic xorshift generator, so randomised tests need no
/// dependency and every failure reproduces from its seed.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        // Mix the seed so that small seeds (1, 2, 3…) give unrelated streams;
        // xorshift must never start from zero.
        Self((seed ^ 0x9E37_79B9_7F4A_7C15).max(1))
    }

    /// Uniform in `0..n` (`0` when `n` is 0).
    pub(crate) fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(n.max(1)).unwrap()).unwrap()
    }

    pub(crate) fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    /// Uniform in `lo..=hi`, quantised to 2¹⁶ steps.
    pub(crate) fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        let t = f32::from(u16::try_from(self.below(65_536)).unwrap()) / 65_535.0;
        lo + (hi - lo) * t
    }

    pub(crate) fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        (!items.is_empty()).then(|| items[self.below(items.len())])
    }
}
