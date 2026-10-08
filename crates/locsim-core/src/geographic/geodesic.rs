//! Geodesics on the WGS84 ellipsoid (Vincenty's direct and inverse formulae).
//!
//! Accuracy is sub-millimetre for all non-antipodal pairs. The inverse
//! solution does not converge for nearly antipodal points; that case is
//! reported as [`GeoError::NotConverged`] rather than approximated.

use super::coordinate::{normalize_bearing, normalize_longitude, Coordinate};
use super::wgs84::{A, B, F};
use super::GeoError;
use std::f64::consts::PI;

/// Largest distance accepted by [`direct`]: half the equatorial circumference,
/// beyond which "the" geodesic to the destination is no longer the shortest path.
pub const MAX_GEODESIC_DISTANCE_M: f64 = PI * A;

/// Relative convergence tolerance. It must be relative: the per-iteration
/// correction scales with line length, so an absolute threshold would accept
/// the uncorrected first guess for millimetre-scale lines.
const CONVERGENCE: f64 = 1e-14;
const MAX_ITERATIONS: usize = 200;

/// Solution of the inverse geodesic problem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geodesic {
    pub distance_m: f64,
    /// Bearing at the start point, `[0, 360)`. `0` for coincident points.
    pub initial_bearing_deg: f64,
    /// Bearing at the end point (direction of travel on arrival), `[0, 360)`.
    pub final_bearing_deg: f64,
    /// Meridian convergence along the line: final minus initial bearing, in
    /// `(-180, 180)`. A direction carried along the geodesic without turning
    /// changes its bearing by exactly this much.
    ///
    /// Computed from a half-angle formula rather than by subtracting the two
    /// bearings, so it stays accurate for arbitrarily short lines, where the
    /// individual bearings are ill-conditioned.
    pub convergence_deg: f64,
}

/// Vincenty series coefficients for a given u² = cos²α (a² − b²) / b².
fn series(cos2_alpha: f64) -> (f64, f64) {
    let u2 = cos2_alpha * (A * A - B * B) / (B * B);
    let big_a = 1.0 + u2 / 16384.0 * (4096.0 + u2 * (-768.0 + u2 * (320.0 - 175.0 * u2)));
    let big_b = u2 / 1024.0 * (256.0 + u2 * (-128.0 + u2 * (74.0 - 47.0 * u2)));
    (big_a, big_b)
}

fn delta_sigma(big_b: f64, sin_sigma: f64, cos_sigma: f64, cos2sm: f64) -> f64 {
    big_b
        * sin_sigma
        * (cos2sm
            + big_b / 4.0
                * (cos_sigma * (-1.0 + 2.0 * cos2sm * cos2sm)
                    - big_b / 6.0
                        * cos2sm
                        * (-3.0 + 4.0 * sin_sigma * sin_sigma)
                        * (-3.0 + 4.0 * cos2sm * cos2sm)))
}

/// Inverse problem: distance and bearings between two points.
pub fn inverse(from: Coordinate, to: Coordinate) -> Result<Geodesic, GeoError> {
    const ZERO: Geodesic = Geodesic {
        distance_m: 0.0,
        initial_bearing_deg: 0.0,
        final_bearing_deg: 0.0,
        convergence_deg: 0.0,
    };

    let l = normalize_longitude(to.longitude() - from.longitude()).to_radians();
    if from.latitude() == to.latitude() && l == 0.0 {
        return Ok(ZERO);
    }

    let u1 = ((1.0 - F) * from.latitude().to_radians().tan()).atan();
    let u2 = ((1.0 - F) * to.latitude().to_radians().tan()).atan();
    let (su1, cu1) = u1.sin_cos();
    let (su2, cu2) = u2.sin_cos();

    let mut lambda = l;
    for _ in 0..MAX_ITERATIONS {
        let (sl, cl) = lambda.sin_cos();
        let x = cu2 * sl;
        let y = cu1 * su2 - su1 * cu2 * cl;
        let sin_sigma = (x * x + y * y).sqrt();
        let cos_sigma = su1 * su2 + cu1 * cu2 * cl;
        if sin_sigma == 0.0 {
            // Coincident (e.g. same pole, different longitude) or exactly antipodal.
            return if cos_sigma > 0.0 {
                Ok(ZERO)
            } else {
                Err(GeoError::NotConverged)
            };
        }
        let sigma = sin_sigma.atan2(cos_sigma);
        let sin_alpha = cu1 * cu2 * sl / sin_sigma;
        let cos2_alpha = 1.0 - sin_alpha * sin_alpha;
        // Equatorial line: cos²α = 0, and the term multiplied by it vanishes.
        let cos2sm = if cos2_alpha.abs() < 1e-300 {
            0.0
        } else {
            cos_sigma - 2.0 * su1 * su2 / cos2_alpha
        };
        let c = F / 16.0 * cos2_alpha * (4.0 + F * (4.0 - 3.0 * cos2_alpha));
        let lambda_new = l
            + (1.0 - c)
                * F
                * sin_alpha
                * (sigma
                    + c * sin_sigma * (cos2sm + c * cos_sigma * (-1.0 + 2.0 * cos2sm * cos2sm)));
        if !lambda_new.is_finite() || lambda_new.abs() > PI {
            return Err(GeoError::NotConverged);
        }
        let converged = (lambda_new - lambda).abs() <= CONVERGENCE * lambda_new.abs();
        lambda = lambda_new;
        if converged {
            let (big_a, big_b) = series(cos2_alpha);
            let distance_m = B * big_a * (sigma - delta_sigma(big_b, sin_sigma, cos_sigma, cos2sm));
            let (sl, cl) = lambda.sin_cos();
            let az1 = (cu2 * sl).atan2(cu1 * su2 - su1 * cu2 * cl);
            let az2 = (cu1 * sl).atan2(-su1 * cu2 + cu1 * su2 * cl);
            if !distance_m.is_finite() || distance_m < 0.0 {
                return Err(GeoError::NotConverged);
            }
            // Napier's analogy on the auxiliary sphere (reduced latitudes
            // u1, u2 and longitude difference λ), where az1 and az2 live:
            // tan(γ/2) = sin((u1+u2)/2) · tan(λ/2) / cos((u2−u1)/2).
            let half = 0.5 * lambda;
            let convergence = 2.0
                * ((0.5 * (u1 + u2)).sin() * half.sin())
                    .atan2((0.5 * (u2 - u1)).cos() * half.cos());
            return Ok(Geodesic {
                distance_m,
                initial_bearing_deg: normalize_bearing(az1.to_degrees()),
                final_bearing_deg: normalize_bearing(az2.to_degrees()),
                convergence_deg: convergence.to_degrees(),
            });
        }
    }
    Err(GeoError::NotConverged)
}

/// Direct problem: the point reached by travelling `distance_m` from `start`
/// on initial bearing `bearing_deg`. Returns the destination and the bearing
/// on arrival. Longitude is wrapped; passing over a pole is handled.
pub fn direct(
    start: Coordinate,
    bearing_deg: f64,
    distance_m: f64,
) -> Result<(Coordinate, f64), GeoError> {
    if !bearing_deg.is_finite() {
        return Err(GeoError::InvalidInput("bearing must be finite"));
    }
    if !distance_m.is_finite() || !(0.0..=MAX_GEODESIC_DISTANCE_M).contains(&distance_m) {
        return Err(GeoError::InvalidInput(
            "distance must be within [0, MAX_GEODESIC_DISTANCE_M]",
        ));
    }
    let bearing_deg = normalize_bearing(bearing_deg);
    if distance_m == 0.0 {
        return Ok((start, bearing_deg));
    }

    let (sa1, ca1) = bearing_deg.to_radians().sin_cos();
    let tan_u1 = (1.0 - F) * start.latitude().to_radians().tan();
    let cu1 = 1.0 / (1.0 + tan_u1 * tan_u1).sqrt();
    let su1 = tan_u1 * cu1;
    let sigma1 = tan_u1.atan2(ca1);
    let sin_alpha = cu1 * sa1;
    let cos2_alpha = 1.0 - sin_alpha * sin_alpha;
    let (big_a, big_b) = series(cos2_alpha);

    let sigma0 = distance_m / (B * big_a);
    let mut sigma = sigma0;
    let mut converged = false;
    for _ in 0..MAX_ITERATIONS {
        let cos2sm = (2.0 * sigma1 + sigma).cos();
        let (sin_sigma, cos_sigma) = sigma.sin_cos();
        let sigma_new = sigma0 + delta_sigma(big_b, sin_sigma, cos_sigma, cos2sm);
        converged = (sigma_new - sigma).abs() <= CONVERGENCE * sigma_new.abs();
        sigma = sigma_new;
        if converged {
            break;
        }
    }
    if !converged {
        return Err(GeoError::NotConverged);
    }
    let cos2sm = (2.0 * sigma1 + sigma).cos();
    let (sin_sigma, cos_sigma) = sigma.sin_cos();

    let tmp = su1 * sin_sigma - cu1 * cos_sigma * ca1;
    let phi2 = (su1 * cos_sigma + cu1 * sin_sigma * ca1)
        .atan2((1.0 - F) * (sin_alpha * sin_alpha + tmp * tmp).sqrt());
    let lambda = (sin_sigma * sa1).atan2(cu1 * cos_sigma - su1 * sin_sigma * ca1);
    let c = F / 16.0 * cos2_alpha * (4.0 + F * (4.0 - 3.0 * cos2_alpha));
    let l = lambda
        - (1.0 - c)
            * F
            * sin_alpha
            * (sigma + c * sin_sigma * (cos2sm + c * cos_sigma * (-1.0 + 2.0 * cos2sm * cos2sm)));
    let final_bearing = normalize_bearing(sin_alpha.atan2(-tmp).to_degrees());

    // phi2 is in [-π/2, π/2] by construction; the clamp only absorbs a
    // possible 1-ulp excess from the degree conversion.
    let lat = phi2.to_degrees().clamp(-90.0, 90.0);
    let lon = normalize_longitude(start.longitude() + l.to_degrees());
    Ok((Coordinate::new(lat, lon)?, final_bearing))
}

/// Geodesic distance in metres.
pub fn distance(from: Coordinate, to: Coordinate) -> Result<f64, GeoError> {
    inverse(from, to).map(|g| g.distance_m)
}

/// Initial bearing from `from` towards `to`, `[0, 360)`.
pub fn bearing(from: Coordinate, to: Coordinate) -> Result<f64, GeoError> {
    inverse(from, to).map(|g| g.initial_bearing_deg)
}

/// Destination point only (see [`direct`]).
pub fn destination(
    start: Coordinate,
    bearing_deg: f64,
    distance_m: f64,
) -> Result<Coordinate, GeoError> {
    direct(start, bearing_deg, distance_m).map(|(c, _)| c)
}

/// Point at `fraction ∈ [0, 1]` of the way along the geodesic from `from` to `to`.
pub fn interpolate(
    from: Coordinate,
    to: Coordinate,
    fraction: f64,
) -> Result<Coordinate, GeoError> {
    if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
        return Err(GeoError::InvalidInput("fraction must be within [0, 1]"));
    }
    if fraction == 0.0 {
        return Ok(from);
    }
    if fraction == 1.0 {
        return Ok(to);
    }
    let g = inverse(from, to)?;
    if g.distance_m == 0.0 {
        return Ok(from);
    }
    destination(from, g.initial_bearing_deg, g.distance_m * fraction)
}

/// Whether `point` lies within `radius_m` (inclusive) of `center`.
pub fn within_radius(
    center: Coordinate,
    point: Coordinate,
    radius_m: f64,
) -> Result<bool, GeoError> {
    if !radius_m.is_finite() || radius_m < 0.0 {
        return Err(GeoError::InvalidInput("radius must be finite and >= 0"));
    }
    Ok(distance(center, point)? <= radius_m)
}

/// Ground speed (m/s) and course (degrees) implied by moving from `from` to
/// `to` in `dt_s` seconds. Course is `None` when there is no displacement,
/// because a stationary fix has no meaningful direction of travel.
pub fn velocity_between(
    from: Coordinate,
    to: Coordinate,
    dt_s: f64,
) -> Result<(f64, Option<f64>), GeoError> {
    if !dt_s.is_finite() || dt_s <= 0.0 {
        return Err(GeoError::InvalidInput("dt must be finite and > 0"));
    }
    let g = inverse(from, to)?;
    if g.distance_m == 0.0 {
        return Ok((0.0, None));
    }
    Ok((g.distance_m / dt_s, Some(g.initial_bearing_deg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(lat: f64, lon: f64) -> Coordinate {
        Coordinate::new(lat, lon).unwrap()
    }

    fn dms(d: f64, m: f64, s: f64) -> f64 {
        d + m / 60.0 + s / 3600.0
    }

    // Vincenty's published test line (Flinders Peak → Buninyong, on GRS80;
    // the WGS84 difference is far below the tolerances used here).
    fn flinders() -> Coordinate {
        c(-dms(37.0, 57.0, 3.72030), dms(144.0, 25.0, 29.52440))
    }
    fn buninyong() -> Coordinate {
        c(-dms(37.0, 39.0, 10.15610), dms(143.0, 55.0, 35.38390))
    }

    #[test]
    fn inverse_matches_published_line() {
        let g = inverse(flinders(), buninyong()).unwrap();
        assert!((g.distance_m - 54_972.271).abs() < 1e-3, "{}", g.distance_m);
        assert!((g.initial_bearing_deg - dms(306.0, 52.0, 5.37)).abs() < 1e-5);
        assert!((g.final_bearing_deg - dms(307.0, 10.0, 25.07)).abs() < 1e-5);
    }

    #[test]
    fn direct_matches_published_line() {
        let (p, fb) = direct(flinders(), dms(306.0, 52.0, 5.37), 54_972.271).unwrap();
        let b = buninyong();
        assert!((p.latitude() - b.latitude()).abs() < 1e-7);
        assert!((p.longitude() - b.longitude()).abs() < 1e-7);
        assert!((fb - dms(307.0, 10.0, 25.07)).abs() < 1e-5);
    }

    #[test]
    fn equatorial_degree_and_meridian_quadrant() {
        let g = inverse(c(0.0, 0.0), c(0.0, 1.0)).unwrap();
        assert!((g.distance_m - A * PI / 180.0).abs() < 1e-6);
        assert!((g.initial_bearing_deg - 90.0).abs() < 1e-9);

        let q = inverse(c(0.0, 0.0), c(90.0, 0.0)).unwrap();
        assert!(
            (q.distance_m - 10_001_965.729).abs() < 1e-3,
            "{}",
            q.distance_m
        );
        assert!(q.initial_bearing_deg.abs() < 1e-9);
    }

    #[test]
    fn coincident_points_are_zero() {
        let p = c(12.9352, 77.6245);
        let g = inverse(p, p).unwrap();
        assert_eq!(g.distance_m, 0.0);
        assert_eq!(velocity_between(p, p, 1.0).unwrap(), (0.0, None));
        assert_eq!(interpolate(p, p, 0.5).unwrap(), p);
        // The same pole reached via different longitudes is one point.
        assert!(distance(c(90.0, 0.0), c(90.0, 120.0)).unwrap() < 1e-6);
        // ±180° are the same meridian.
        assert!(distance(c(10.0, 180.0), c(10.0, -180.0)).unwrap() < 1e-9);
    }

    #[test]
    fn small_distances_are_accurate() {
        let p = c(12.9352, 77.6245);
        for d in [0.001, 0.01, 0.5, 1.0, 10.0] {
            for b in [0.0, 37.0, 90.0, 181.0, 300.0] {
                let q = destination(p, b, d).unwrap();
                let g = inverse(p, q).unwrap();
                assert!(
                    (g.distance_m - d).abs() < 1e-6,
                    "d={d} got {}",
                    g.distance_m
                );
            }
        }
    }

    #[test]
    fn antimeridian_crossing_wraps() {
        let p = c(0.0, 179.9995);
        let q = destination(p, 90.0, 1000.0).unwrap();
        assert!(q.longitude() < -179.0, "{}", q.longitude());
        assert!((distance(p, q).unwrap() - 1000.0).abs() < 1e-6);
        assert!((bearing(p, q).unwrap() - 90.0).abs() < 1e-6);
    }

    #[test]
    fn travelling_over_the_pole() {
        // 1 km short of the north pole heading north for 2 km ends up on the
        // opposite meridian, heading south.
        let near = destination(c(90.0, 0.0), 180.0, 1000.0).unwrap();
        let (over, fb) = direct(near, 0.0, 2000.0).unwrap();
        assert!((distance(c(90.0, 0.0), over).unwrap() - 1000.0).abs() < 1e-5);
        assert!((bearing_gap(over.longitude(), near.longitude()) - 180.0).abs() < 1e-6);
        assert!((fb - 180.0).abs() < 1e-6);
        // Starting exactly at a pole yields a valid point at the right distance.
        for b in [0.0, 90.0, 222.0] {
            let p = destination(c(-90.0, 0.0), b, 5000.0).unwrap();
            assert!((distance(c(-90.0, 0.0), p).unwrap() - 5000.0).abs() < 1e-5);
        }
    }

    fn bearing_gap(a: f64, b: f64) -> f64 {
        normalize_longitude(a - b).abs()
    }

    #[test]
    fn antipodal_reports_non_convergence() {
        assert_eq!(
            inverse(c(0.0, 0.0), c(0.0, 180.0)),
            Err(GeoError::NotConverged)
        );
        assert_eq!(
            inverse(c(0.0, 0.0), c(0.5, 179.7)),
            Err(GeoError::NotConverged)
        );
    }

    #[test]
    fn large_segments() {
        // London → Sydney, ~17 000 km: direct must reproduce the inverse.
        let (a, b) = (c(51.5074, -0.1278), c(-33.8688, 151.2093));
        let g = inverse(a, b).unwrap();
        assert!(g.distance_m > 16_900_000.0 && g.distance_m < 17_100_000.0);
        let (p, fb) = direct(a, g.initial_bearing_deg, g.distance_m).unwrap();
        assert!(distance(p, b).unwrap() < 1e-3);
        assert!((fb - g.final_bearing_deg).abs() < 1e-6);
    }

    #[test]
    fn interpolation_endpoints_and_midpoint() {
        let (a, b) = (c(12.9352, 77.6245), c(13.0827, 80.2707));
        assert_eq!(interpolate(a, b, 0.0).unwrap(), a);
        assert_eq!(interpolate(a, b, 1.0).unwrap(), b);
        let m = interpolate(a, b, 0.5).unwrap();
        let total = distance(a, b).unwrap();
        assert!((distance(a, m).unwrap() - total / 2.0).abs() < 1e-5);
        assert!((distance(m, b).unwrap() - total / 2.0).abs() < 1e-5);
    }

    #[test]
    fn convergence_matches_bearing_difference_and_direct() {
        use crate::geographic::bearing_difference;
        let mut rng = crate::rng::Rng::from_seed(0xC0);
        for _ in 0..5_000 {
            let a = c(rng.uniform(-89.5, 89.5), rng.uniform(-180.0, 180.0));
            let heading = rng.uniform(0.0, 360.0);
            let d = 10f64.powf(rng.uniform(1.0, 6.0));
            let (b, arrival) = direct(a, heading, d).unwrap();
            let g = inverse(a, b).unwrap();
            // Agrees with the plain difference where that is well-conditioned…
            let plain = bearing_difference(g.initial_bearing_deg, g.final_bearing_deg);
            assert!(
                (g.convergence_deg - plain).abs() < 1e-6,
                "{a:?} {heading} {d}"
            );
            // …and with the direct solution's change of bearing.
            let from_direct = bearing_difference(heading, arrival);
            assert!((g.convergence_deg - from_direct).abs() < 1e-6);
        }
        // Sign: heading east in the northern hemisphere, bearing increases.
        assert!(
            inverse(c(60.0, 0.0), c(60.0, 10.0))
                .unwrap()
                .convergence_deg
                > 0.0
        );
        assert!(
            inverse(c(-60.0, 0.0), c(-60.0, 10.0))
                .unwrap()
                .convergence_deg
                < 0.0
        );
        assert_eq!(
            inverse(c(10.0, 20.0), c(10.0, 20.0))
                .unwrap()
                .convergence_deg,
            0.0
        );
    }

    #[test]
    fn convergence_stays_accurate_for_sub_millimetre_lines_at_high_latitude() {
        use crate::geographic::bearing_difference;
        // 0.1 mm steps 1.1 km from the pole: the two bearings from `inverse`
        // are each only good to ~1e-3°, the convergence to better than 1e-9°.
        let a = c(89.99, 45.0);
        for heading in [10.0, 80.0, 135.0, 250.0, 359.0] {
            let (b, arrival) = direct(a, heading, 1e-4).unwrap();
            let g = inverse(a, b).unwrap();
            let expected = bearing_difference(heading, arrival);
            assert!(
                (g.convergence_deg - expected).abs() < 1e-9,
                "{heading}: {} vs {expected}",
                g.convergence_deg
            );
        }
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        let p = c(0.0, 0.0);
        assert!(direct(p, f64::NAN, 1.0).is_err());
        assert!(direct(p, 0.0, -1.0).is_err());
        assert!(direct(p, 0.0, f64::INFINITY).is_err());
        assert!(direct(p, 0.0, MAX_GEODESIC_DISTANCE_M * 1.01).is_err());
        assert!(interpolate(p, p, 1.5).is_err());
        assert!(interpolate(p, p, f64::NAN).is_err());
        assert!(within_radius(p, p, -1.0).is_err());
        assert!(velocity_between(p, p, 0.0).is_err());
        assert!(velocity_between(p, p, -1.0).is_err());
    }

    #[test]
    fn velocity_and_radius() {
        let p = c(12.9352, 77.6245);
        let q = destination(p, 45.0, 30.0).unwrap();
        let (speed, course) = velocity_between(p, q, 2.0).unwrap();
        assert!((speed - 15.0).abs() < 1e-6);
        assert!((course.unwrap() - 45.0).abs() < 1e-4);
        assert!(within_radius(p, q, 30.001).unwrap());
        assert!(!within_radius(p, q, 29.999).unwrap());
    }
}
