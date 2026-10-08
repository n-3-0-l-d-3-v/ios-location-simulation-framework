//! Deterministic pseudo-random number generator.
//!
//! xoshiro256** seeded through SplitMix64. Implemented in-crate (rather than
//! depending on `rand`) so that a `(scenario, seed)` pair yields a bit-identical
//! sample stream on every platform and toolchain. Not cryptographically secure.

/// SplitMix64 step; used to expand a 64-bit seed into generator state.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Seeded deterministic generator (xoshiro256**).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn from_seed(seed: u64) -> Self {
        let mut sm = seed;
        let s = [
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
        ];
        // SplitMix64 is a bijection over a non-repeating counter, so four
        // consecutive outputs can never all be zero.
        Self { s }
    }

    /// Derives an independent child stream, so separate consumers (movement,
    /// noise, …) do not perturb each other's sequences.
    pub fn fork(&mut self, stream: u64) -> Self {
        Self::from_seed(self.next_u64() ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, 1)` with 53 bits of precision.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// Uniform in `[low, high)`. Returns `low` when the range is empty.
    pub fn uniform(&mut self, low: f64, high: f64) -> f64 {
        if high <= low {
            return low;
        }
        let v = low + (high - low) * self.next_f64();
        // Guard against rounding up to `high` for very narrow ranges.
        if v >= high {
            low
        } else {
            v
        }
    }

    /// Standard normal deviate (Box–Muller; one deviate per call so the
    /// stream position is independent of call history).
    pub fn standard_normal(&mut self) -> f64 {
        let u1 = 1.0 - self.next_f64(); // (0, 1]
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    pub fn gaussian(&mut self, mean: f64, std_dev: f64) -> f64 {
        mean + std_dev * self.standard_normal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_matches_reference_vector() {
        let mut s = 1_234_567u64;
        assert_eq!(splitmix64(&mut s), 6_457_827_717_110_365_317);
        assert_eq!(splitmix64(&mut s), 3_203_168_211_198_807_973);
    }

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::from_seed(42);
        let mut b = Rng::from_seed(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::from_seed(1);
        let mut b = Rng::from_seed(2);
        assert!((0..8).any(|_| a.next_u64() != b.next_u64()));
    }

    #[test]
    fn forks_are_deterministic_and_distinct() {
        let mut p1 = Rng::from_seed(7);
        let mut p2 = Rng::from_seed(7);
        let mut c1 = p1.fork(1);
        let mut c2 = p2.fork(1);
        assert_eq!(c1.next_u64(), c2.next_u64());
        let mut d = Rng::from_seed(7).fork(2);
        let mut e = Rng::from_seed(7).fork(1);
        assert_ne!(d.next_u64(), e.next_u64());
    }

    #[test]
    fn next_f64_in_unit_interval_with_plausible_mean() {
        let mut r = Rng::from_seed(99);
        let n = 100_000;
        let mut sum = 0.0;
        for _ in 0..n {
            let v = r.next_f64();
            assert!((0.0..1.0).contains(&v));
            sum += v;
        }
        assert!((sum / n as f64 - 0.5).abs() < 0.005);
    }

    #[test]
    fn uniform_respects_bounds() {
        let mut r = Rng::from_seed(3);
        for _ in 0..10_000 {
            let v = r.uniform(-2.5, 7.0);
            assert!((-2.5..7.0).contains(&v));
        }
        assert_eq!(r.uniform(4.0, 4.0), 4.0);
        assert_eq!(r.uniform(4.0, 1.0), 4.0);
    }

    #[test]
    fn standard_normal_moments() {
        let mut r = Rng::from_seed(2024);
        let n = 200_000;
        let (mut sum, mut sq) = (0.0, 0.0);
        for _ in 0..n {
            let v = r.standard_normal();
            assert!(v.is_finite());
            sum += v;
            sq += v * v;
        }
        let mean = sum / n as f64;
        let var = sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.01, "mean {mean}");
        assert!((var - 1.0).abs() < 0.02, "var {var}");
    }
}
