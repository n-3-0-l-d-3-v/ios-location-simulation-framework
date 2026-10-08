//! Property tests for the geographic engine over seeded random inputs.
//! Each property runs `CASES` times; failures print the case index so the
//! exact input can be regenerated from the fixed seed.

use locsim_core::geographic::{
    bearing_difference, destination, direct, distance, interpolate, inverse, normalize_bearing,
    normalize_longitude, Coordinate, EnuFrame,
};
use locsim_core::rng::Rng;

const CASES: usize = 20_000;

fn random_coordinate(rng: &mut Rng, max_abs_lat: f64) -> Coordinate {
    Coordinate::new(
        rng.uniform(-max_abs_lat, max_abs_lat),
        rng.uniform(-180.0, 180.0),
    )
    .unwrap()
}

fn assert_valid(c: Coordinate, case: usize) {
    assert!(
        c.latitude().is_finite() && c.longitude().is_finite(),
        "case {case}: {c:?}"
    );
    assert!((-90.0..=90.0).contains(&c.latitude()), "case {case}: {c:?}");
    assert!(
        (-180.0..=180.0).contains(&c.longitude()),
        "case {case}: {c:?}"
    );
}

#[test]
fn direct_always_yields_valid_coordinate_and_bearing() {
    let mut rng = Rng::from_seed(0xA11CE);
    for case in 0..CASES {
        // Full latitude range, including the poles themselves now and then.
        let lat = match case % 50 {
            0 => 90.0,
            1 => -90.0,
            _ => rng.uniform(-90.0, 90.0),
        };
        let start = Coordinate::new(lat, rng.uniform(-180.0, 180.0)).unwrap();
        let bearing = rng.uniform(-720.0, 720.0);
        let dist = 10f64.powf(rng.uniform(-3.0, 7.2)); // 1 mm … ~15 800 km
        let (p, fb) = direct(start, bearing, dist).unwrap();
        assert_valid(p, case);
        assert!(
            (0.0..360.0).contains(&fb),
            "case {case}: final bearing {fb}"
        );
    }
}

#[test]
fn direct_then_inverse_round_trips() {
    let mut rng = Rng::from_seed(0xB0B);
    for case in 0..CASES {
        let start = random_coordinate(&mut rng, 89.0);
        let bearing = rng.uniform(0.0, 360.0);
        let dist = 10f64.powf(rng.uniform(0.0, 6.5)); // 1 m … ~3 160 km
        let end = destination(start, bearing, dist).unwrap();
        let g = inverse(start, end).unwrap();
        assert!(
            (g.distance_m - dist).abs() < 1e-5,
            "case {case}: {dist} vs {}",
            g.distance_m
        );
        // Bearing resolution degrades as 1/distance; allow 1 µm of cross-track.
        let tol = (1e-6 / dist).to_degrees().max(1e-8);
        assert!(
            bearing_difference(bearing, g.initial_bearing_deg).abs() < tol,
            "case {case}: {bearing} vs {} (d={dist})",
            g.initial_bearing_deg
        );
    }
}

#[test]
fn inverse_is_symmetric_and_reverses_bearing() {
    let mut rng = Rng::from_seed(0xC0FFEE);
    for case in 0..CASES {
        let a = random_coordinate(&mut rng, 89.0);
        let dist = 10f64.powf(rng.uniform(1.0, 6.5));
        let b = destination(a, rng.uniform(0.0, 360.0), dist).unwrap();
        let ab = inverse(a, b).unwrap();
        let ba = inverse(b, a).unwrap();
        assert!((ab.distance_m - ba.distance_m).abs() < 1e-5, "case {case}");
        // Arriving bearing a→b is the reverse of the departing bearing b→a.
        let back = normalize_bearing(ab.final_bearing_deg + 180.0);
        let tol = (1e-6 / dist).to_degrees().max(1e-8);
        assert!(
            bearing_difference(back, ba.initial_bearing_deg).abs() < tol,
            "case {case}"
        );
    }
}

#[test]
fn interpolation_lies_on_the_geodesic() {
    let mut rng = Rng::from_seed(0xD00D);
    for case in 0..CASES / 4 {
        let a = random_coordinate(&mut rng, 85.0);
        let b = destination(
            a,
            rng.uniform(0.0, 360.0),
            10f64.powf(rng.uniform(0.0, 6.0)),
        )
        .unwrap();
        let t = rng.next_f64();
        let m = interpolate(a, b, t).unwrap();
        assert_valid(m, case);
        let total = distance(a, b).unwrap();
        let (am, mb) = (distance(a, m).unwrap(), distance(m, b).unwrap());
        assert!((am - total * t).abs() < 1e-4, "case {case}");
        assert!((am + mb - total).abs() < 1e-4, "case {case}");
    }
}

#[test]
fn enu_round_trips_and_agrees_with_geodesic_locally() {
    let mut rng = Rng::from_seed(0xE66);
    for case in 0..CASES {
        let origin = random_coordinate(&mut rng, 89.9);
        let frame = EnuFrame::new(origin, rng.uniform(-100.0, 3000.0)).unwrap();
        let (east, north) = (rng.uniform(-1000.0, 1000.0), rng.uniform(-1000.0, 1000.0));

        let p = frame.horizontal_to_coordinate(east, north).unwrap();
        assert_valid(p, case);
        // Same altitude as the origin: the horizontal offset must survive.
        let (coord, alt) = frame
            .from_enu(locsim_core::geographic::Enu {
                east,
                north,
                up: 0.0,
            })
            .unwrap();
        let back = frame.to_enu(coord, alt).unwrap();
        assert!((back.east - east).abs() < 1e-5, "case {case}");
        assert!((back.north - north).abs() < 1e-5, "case {case}");
        assert!(back.up.abs() < 1e-5, "case {case}");

        // Within 1.5 km the planar distance matches the geodesic to < 1 mm
        // per km of altitude-induced scale; allow a relative 1e-3.
        let planar = east.hypot(north);
        let geodesic = distance(origin, p).unwrap();
        assert!(
            (planar - geodesic).abs() <= 1e-3 * planar + 1e-4,
            "case {case}: planar {planar} geodesic {geodesic}"
        );
    }
}

#[test]
fn normalisation_ranges_hold() {
    let mut rng = Rng::from_seed(0xF00);
    for _ in 0..CASES {
        let v = rng.uniform(-1e6, 1e6);
        let lon = normalize_longitude(v);
        assert!((-180.0..=180.0).contains(&lon));
        // Wrapping must not move the point: difference is a multiple of 360.
        let turns = (v - lon) / 360.0;
        assert!((turns - turns.round()).abs() < 1e-6);
        assert!((0.0..360.0).contains(&normalize_bearing(v)));
        let d = bearing_difference(v, rng.uniform(-1e6, 1e6));
        assert!(d > -180.0 - 1e-9 && d <= 180.0);
    }
}
