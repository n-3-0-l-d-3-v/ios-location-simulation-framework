//! Small polynomial toolkit for bounding quantities along a spline segment.
//!
//! Admission must decide whether a quantity stays under a limit *everywhere*
//! on a segment, not merely at sampled instants. Every quantity involved is
//! a polynomial in the segment parameter, or a ratio of polynomials, so its
//! range over an interval can be bounded rigorously: a polynomial lies within
//! the convex hull of its Bernstein coefficients. [`supremum`] then bisects
//! until that upper bound and an actually attained value agree.

pub(crate) type Vec3 = [f64; 3];

pub(crate) fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

pub(crate) fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: Vec3, k: f64) -> Vec3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub(crate) const MAX_DEGREE: usize = 6;

/// Pascal's triangle up to `MAX_DEGREE`.
const BINOMIAL: [[f64; MAX_DEGREE + 1]; MAX_DEGREE + 1] = [
    [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [1.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0],
    [1.0, 3.0, 3.0, 1.0, 0.0, 0.0, 0.0],
    [1.0, 4.0, 6.0, 4.0, 1.0, 0.0, 0.0],
    [1.0, 5.0, 10.0, 10.0, 5.0, 1.0, 0.0],
    [1.0, 6.0, 15.0, 20.0, 15.0, 6.0, 1.0],
];

/// Scalar polynomial of degree ≤ [`MAX_DEGREE`], power basis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Poly {
    c: [f64; MAX_DEGREE + 1],
    degree: usize,
}

impl Poly {
    pub(crate) fn new(coefficients: &[f64]) -> Self {
        assert!(!coefficients.is_empty() && coefficients.len() <= MAX_DEGREE + 1);
        let mut c = [0.0; MAX_DEGREE + 1];
        c[..coefficients.len()].copy_from_slice(coefficients);
        Self {
            c,
            degree: coefficients.len() - 1,
        }
    }

    pub(crate) fn eval(&self, x: f64) -> f64 {
        self.c[..=self.degree]
            .iter()
            .rev()
            .fold(0.0, |acc, c| acc * x + c)
    }

    /// Lower and upper bounds on the polynomial over `[a, b]`.
    ///
    /// The polynomial is re-expressed on the interval and converted to the
    /// Bernstein basis, whose coefficients enclose its range. The bounds are
    /// exact at the interval ends and tighten quadratically as it shrinks.
    pub(crate) fn range(&self, a: f64, b: f64) -> (f64, f64) {
        let n = self.degree;
        let mut q = self.c;
        // Taylor shift: q(s) = p(a + s).
        for i in 0..n {
            for j in (i..n).rev() {
                q[j] += a * q[j + 1];
            }
        }
        // Scale: q(s) = p(a + (b − a) s), s ∈ [0, 1].
        let width = b - a;
        let mut power = 1.0;
        for coefficient in q.iter_mut().take(n + 1) {
            *coefficient *= power;
            power *= width;
        }
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for (i, row) in BINOMIAL.iter().enumerate().take(n + 1) {
            let bernstein: f64 = (0..=i).map(|j| row[j] / BINOMIAL[n][j] * q[j]).sum();
            low = low.min(bernstein);
            high = high.max(bernstein);
        }
        (low, high)
    }
}

/// `(Σ aᵢ xⁱ) · (Σ bⱼ xʲ)` for vector-valued coefficients.
pub(crate) fn dot_poly(a: &[Vec3], b: &[Vec3]) -> Poly {
    let degree = a.len() + b.len() - 2;
    assert!(degree <= MAX_DEGREE);
    let mut c = [0.0; MAX_DEGREE + 1];
    for (i, ai) in a.iter().enumerate() {
        for (j, bj) in b.iter().enumerate() {
            c[i + j] += dot(*ai, *bj);
        }
    }
    Poly { c, degree }
}

/// Result of [`supremum`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Supremum {
    /// An upper bound on the function over `[0, 1]`: never below the true
    /// supremum, and at most a relative 1e-7 (plus the caller's absolute
    /// floor) above it unless the search was cut short, in which case it is
    /// larger still (possibly infinite).
    pub(crate) value: f64,
    /// Where the bound is attained or, if unresolved, where it could not be
    /// brought down.
    pub(crate) at: f64,
}

/// How close the certified bound must come to an attained value. A ratio of
/// polynomials is bounded by bounding numerator and denominator separately,
/// which converges only linearly near a smooth maximum: resolving a gap `g`
/// takes about `1/sqrt(g)` intervals. 1e-7 costs a few thousand; it is ten
/// times finer than the margin admission keeps below each limit.
const RELATIVE_GAP: f64 = 1e-7;
const MAX_DEPTH: u32 = 48;
const MAX_NODES: usize = 200_000;

/// Supremum over `[0, 1]` of a non-negative function, given a rigorous
/// `upper(a, b)` bound on any sub-interval and a way to evaluate it.
///
/// Branch and bound: an interval is discarded once its upper bound is no
/// better than a value already attained (to within `RELATIVE_GAP`, or
/// `floor` in absolute terms); otherwise it is bisected. Whatever cannot be
/// resolved within the depth and node budgets keeps its upper bound, so the
/// result errs on the side of rejecting.
pub(crate) fn supremum(
    upper: &dyn Fn(f64, f64) -> f64,
    value: &dyn Fn(f64) -> f64,
    floor: f64,
) -> Supremum {
    let mut best = Supremum {
        value: 0.0,
        at: 0.0,
    };
    for x in [0.0, 0.5, 1.0] {
        let v = value(x);
        if v > best.value {
            best = Supremum { value: v, at: x };
        }
    }
    let mut unresolved = Supremum {
        value: 0.0,
        at: 0.0,
    };
    let mut stack = vec![(0.0f64, 1.0f64, 0u32)];
    let mut nodes = 0usize;
    while let Some((a, b, depth)) = stack.pop() {
        nodes += 1;
        let bound = upper(a, b);
        if bound <= best.value * (1.0 + RELATIVE_GAP) + floor {
            continue;
        }
        let mid = 0.5 * (a + b);
        if depth >= MAX_DEPTH || nodes > MAX_NODES || bound.is_nan() {
            let bound = if bound.is_nan() { f64::INFINITY } else { bound };
            if bound > unresolved.value {
                unresolved = Supremum {
                    value: bound,
                    at: mid,
                };
            }
            continue;
        }
        let v = value(mid);
        if v > best.value {
            best = Supremum { value: v, at: mid };
        }
        stack.push((a, mid, depth + 1));
        stack.push((mid, b, depth + 1));
    }
    // Every discarded interval was bounded by this, so it is a true upper
    // bound, at most the gap above a value that is actually attained.
    let certified = best.value * (1.0 + RELATIVE_GAP) + floor;
    if unresolved.value > certified {
        unresolved
    } else {
        Supremum {
            value: certified,
            at: best.at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn evaluates_and_multiplies_vector_polynomials() {
        let p = Poly::new(&[1.0, -3.0, 2.0]); // (2x − 1)(x − 1)
        assert_eq!(p.eval(0.0), 1.0);
        assert_eq!(p.eval(0.5), 0.0);
        assert_eq!(p.eval(1.0), 0.0);
        assert_eq!(p.eval(2.0), 3.0);

        // |(1,0,0) + (0,2,0)x|² = 1 + 4x².
        let v = [[1.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        let s = dot_poly(&v, &v);
        assert_eq!((s.eval(0.0), s.eval(1.0), s.eval(3.0)), (1.0, 5.0, 37.0));
        assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
        assert_eq!(norm([3.0, 4.0, 0.0]), 5.0);
        assert_eq!(
            sub(add([1.0, 2.0, 3.0], [1.0, 1.0, 1.0]), [2.0, 3.0, 4.0]),
            [0.0; 3]
        );
        assert_eq!(scale([1.0, -2.0, 0.5], 2.0), [2.0, -4.0, 1.0]);
    }

    #[test]
    fn range_encloses_every_value_and_is_exact_at_the_ends() {
        let mut rng = Rng::from_seed(0x0601);
        for _ in 0..2_000 {
            let degree = 1 + (rng.next_u64() % MAX_DEGREE as u64) as usize;
            let coefficients: Vec<f64> = (0..=degree).map(|_| rng.uniform(-10.0, 10.0)).collect();
            let p = Poly::new(&coefficients);
            let a = rng.uniform(0.0, 1.0);
            let b = rng.uniform(a, 1.0);
            let (low, high) = p.range(a, b);
            let slack = 1e-9 * (1.0 + low.abs().max(high.abs()));
            for k in 0..=50 {
                let x = a + (b - a) * k as f64 / 50.0;
                let y = p.eval(x);
                assert!(
                    y >= low - slack && y <= high + slack,
                    "{y} outside [{low}, {high}]"
                );
            }
            // The end values are Bernstein coefficients themselves.
            assert!(p.eval(a) >= low - slack && p.eval(b) <= high + slack);
        }
    }

    #[test]
    fn range_tightens_as_the_interval_shrinks() {
        let p = Poly::new(&[0.0, 0.0, 1.0, -2.0, 1.0]); // x²(1 − x)²
        let (_, wide) = p.range(0.0, 1.0);
        let (_, narrow) = p.range(0.49, 0.51);
        let peak = p.eval(0.5);
        assert!(wide >= peak && narrow >= peak);
        assert!(narrow - peak < 1e-3 * (wide - peak));
        // Linear and constant polynomials are bounded exactly.
        assert_eq!(Poly::new(&[2.0, 3.0]).range(0.25, 0.5), (2.75, 3.5));
        assert_eq!(Poly::new(&[7.0]).range(0.0, 1.0), (7.0, 7.0));
    }

    #[test]
    fn supremum_finds_interior_and_boundary_maxima() {
        // x(1 − x): maximum 0.25 at 0.5.
        let p = Poly::new(&[0.0, 1.0, -1.0]);
        let s = supremum(&|a, b| p.range(a, b).1, &|x| p.eval(x), 1e-15);
        assert!((s.value - 0.25).abs() < 2e-7 && (s.at - 0.5).abs() < 1e-6);

        // A sharp off-centre peak that three probes would miss entirely.
        let q = Poly::new(&[0.0, 0.0, 0.0, 0.0, 0.0, 6.0, -5.0]); // 6x⁵ − 5x⁶
        let s = supremum(&|a, b| q.range(a, b).1, &|x| q.eval(x), 1e-15);
        assert!((s.value - 1.0).abs() < 2e-7 && (s.at - 1.0).abs() < 1e-6);

        let mut rng = Rng::from_seed(0x0602);
        for _ in 0..300 {
            let coefficients: Vec<f64> = (0..=4).map(|_| rng.uniform(-5.0, 5.0)).collect();
            let p = Poly::new(&coefficients);
            let squared = |x: f64| p.eval(x).powi(2);
            let upper = |a: f64, b: f64| {
                let (low, high) = p.range(a, b);
                low.abs().max(high.abs()).powi(2)
            };
            let s = supremum(&upper, &squared, 1e-15);
            let brute = (0..=20_000)
                .map(|k| squared(k as f64 / 20_000.0))
                .fold(0.0, f64::max);
            assert!(s.value >= brute, "{} < {brute}", s.value);
            assert!(
                s.value <= brute * (1.0 + 1e-6) + 1e-9,
                "{} vs {brute}",
                s.value
            );
        }
    }

    #[test]
    fn supremum_of_a_ratio_with_a_vanishing_denominator_is_infinite() {
        // 1 / (x − 0.3)²: unbounded inside the interval.
        let den = Poly::new(&[0.09, -0.6, 1.0]);
        let upper = |a: f64, b: f64| {
            let (low, _) = den.range(a, b);
            if low <= 0.0 {
                f64::INFINITY
            } else {
                1.0 / low
            }
        };
        let value = |x: f64| {
            let d = den.eval(x);
            if d > 0.0 {
                1.0 / d
            } else {
                0.0
            }
        };
        let s = supremum(&upper, &value, 1e-15);
        assert!(s.value > 1e12, "{}", s.value);
        assert!((s.at - 0.3).abs() < 1e-3, "{}", s.at);
    }
}
