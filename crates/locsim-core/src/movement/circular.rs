use super::{Kinematics, MovementError, MovementModel, MovementSample};
use crate::domain::{Coordinate, RotationDirection, Scenario, Timestamp};
use crate::geographic::{self, normalize_bearing};

/// Mode E: uniform motion around a geodesic circle.
///
/// The position is a closed-form function of elapsed time — the point at
/// distance `radius` from the centre on bearing `phase ± ω·t` — so nothing is
/// integrated and no error can accumulate, however long the run. The orbit
/// starts in steady state: there is no run-up from rest.
///
/// What is exact and what is nominal: every point is exactly `radius_m`
/// (geodesic) from the centre and the bearing advances at exactly the
/// configured angular velocity. The reported speed is the nominal `r·ω`; the
/// true ground speed is lower by a factor of about `1 − r²/6R²` (2e-8 for a
/// 2 km orbit) because a circle on a curved surface is shorter than `2πr`.
#[derive(Debug, Clone, PartialEq)]
pub struct CircularModel {
    center: Coordinate,
    altitude_m: f64,
    radius_m: f64,
    /// Signed: positive = clockwise seen from above (bearing increasing).
    angular_velocity_dps: f64,
    start_phase_deg: f64,
    start: Option<Timestamp>,
    last: Option<(Timestamp, Kinematics)>,
}

impl CircularModel {
    pub fn new(
        center: Coordinate,
        altitude_m: f64,
        radius_m: f64,
        angular_velocity_dps: f64,
        direction: RotationDirection,
        start_phase_deg: f64,
    ) -> Self {
        let sign = match direction {
            RotationDirection::Clockwise => 1.0,
            RotationDirection::CounterClockwise => -1.0,
        };
        Self {
            center,
            altitude_m,
            radius_m,
            angular_velocity_dps: sign * angular_velocity_dps,
            start_phase_deg,
            start: None,
            last: None,
        }
    }

    pub fn for_scenario(scenario: &Scenario) -> Result<Self, MovementError> {
        let m = &scenario.movement;
        let radius_m = m.radius_m.ok_or(MovementError::InvalidConfiguration(
            "circular mode needs a radius",
        ))?;
        let omega = m
            .angular_velocity_dps
            .ok_or(MovementError::InvalidConfiguration(
                "circular mode needs an angular velocity",
            ))?;
        Ok(Self::new(
            scenario.origin,
            scenario.altitude_m,
            radius_m,
            omega,
            m.direction,
            m.start_phase_deg,
        ))
    }

    /// Seconds for one full revolution.
    pub fn period_s(&self) -> f64 {
        360.0 / self.angular_velocity_dps.abs()
    }
}

impl MovementModel for CircularModel {
    fn sample_at(&mut self, t: Timestamp) -> Result<MovementSample, MovementError> {
        if let Some((previous, _)) = self.last {
            if t <= previous {
                return Err(MovementError::NonIncreasingTime {
                    previous,
                    current: t,
                });
            }
        }
        let start = *self.start.get_or_insert(t);
        let elapsed_s = t.seconds_since(start);
        let bearing =
            normalize_bearing(self.start_phase_deg + self.angular_velocity_dps * elapsed_s);
        let (position, outward) = geographic::direct(self.center, bearing, self.radius_m)?;
        // The direction of travel is perpendicular to the radius, on the
        // side the orbit turns towards.
        let heading = normalize_bearing(outward + 90.0 * self.angular_velocity_dps.signum());
        let speed = self.radius_m * self.angular_velocity_dps.abs().to_radians();

        self.last = Some((
            t,
            Kinematics {
                position,
                speed_mps: speed,
                heading_deg: heading,
                acceleration_mps2: 0.0,
                heading_rate_dps: self.angular_velocity_dps,
            },
        ));
        Ok(MovementSample {
            coordinate: position,
            altitude_m: self.altitude_m,
            speed_mps: Some(speed),
            course_deg: (speed > 0.0).then_some(heading),
        })
    }

    fn kinematics(&self) -> Option<Kinematics> {
        self.last.map(|(_, k)| k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geographic::{bearing_difference, distance, inverse};

    const SEC: i64 = 1_000_000_000;

    fn t(seconds: f64) -> Timestamp {
        Timestamp::from_nanos((seconds * 1e9) as i64)
    }

    fn center() -> Coordinate {
        Coordinate::new(12.9352, 77.6245).unwrap()
    }

    fn orbit(direction: RotationDirection, phase: f64) -> CircularModel {
        // 100 m radius, one revolution per 120 s.
        CircularModel::new(center(), 50.0, 100.0, 3.0, direction, phase)
    }

    #[test]
    fn every_point_is_on_the_circle_at_the_expected_bearing() {
        let mut m = orbit(RotationDirection::Clockwise, 30.0);
        assert_eq!(m.period_s(), 120.0);
        for k in 0..=240 {
            let s = m.sample_at(t(k as f64 * 0.5)).unwrap();
            let g = inverse(center(), s.coordinate).unwrap();
            assert!((g.distance_m - 100.0).abs() < 1e-6, "{}", g.distance_m);
            let expected = normalize_bearing(30.0 + 3.0 * k as f64 * 0.5);
            assert!(bearing_difference(expected, g.initial_bearing_deg).abs() < 1e-6);
            assert_eq!(s.altitude_m, 50.0);
        }
    }

    #[test]
    fn starts_at_the_phase_and_returns_after_one_period() {
        let mut m = orbit(RotationDirection::Clockwise, 90.0);
        let first = m.sample_at(t(0.0)).unwrap();
        // Phase 90° = due east of the centre, heading south (clockwise).
        let g = inverse(center(), first.coordinate).unwrap();
        assert!((g.initial_bearing_deg - 90.0).abs() < 1e-6);
        assert!(bearing_difference(first.course_deg.unwrap(), 180.0).abs() < 1e-3);
        let quarter = m.sample_at(t(30.0)).unwrap();
        let g = inverse(center(), quarter.coordinate).unwrap();
        assert!((g.initial_bearing_deg - 180.0).abs() < 1e-6);
        let full = m.sample_at(t(120.0)).unwrap();
        assert!(distance(first.coordinate, full.coordinate).unwrap() < 1e-6);
    }

    #[test]
    fn direction_reverses_the_sense_of_rotation() {
        let mut cw = orbit(RotationDirection::Clockwise, 0.0);
        let mut ccw = orbit(RotationDirection::CounterClockwise, 0.0);
        cw.sample_at(t(0.0)).unwrap();
        ccw.sample_at(t(0.0)).unwrap();
        let a = cw.sample_at(t(10.0)).unwrap();
        let b = ccw.sample_at(t(10.0)).unwrap();
        let ga = inverse(center(), a.coordinate).unwrap();
        let gb = inverse(center(), b.coordinate).unwrap();
        assert!((ga.initial_bearing_deg - 30.0).abs() < 1e-6);
        assert!((gb.initial_bearing_deg - 330.0).abs() < 1e-6);
        // Clockwise at bearing 30° heads 120°; the other way heads 240°.
        assert!(bearing_difference(a.course_deg.unwrap(), 120.0).abs() < 1e-3);
        assert!(bearing_difference(b.course_deg.unwrap(), 240.0).abs() < 1e-3);
        assert_eq!(cw.kinematics().unwrap().heading_rate_dps, 3.0);
        assert_eq!(ccw.kinematics().unwrap().heading_rate_dps, -3.0);
    }

    #[test]
    fn speed_is_constant_and_matches_the_ground_track() {
        let mut m = orbit(RotationDirection::Clockwise, 0.0);
        let nominal = 100.0 * 3.0f64.to_radians();
        let mut previous = m.sample_at(t(0.0)).unwrap();
        for k in 1..2_000 {
            let s = m.sample_at(t(k as f64 * 0.01)).unwrap();
            assert_eq!(s.speed_mps, Some(nominal));
            // Chord over 10 ms vs nominal arc: equal to within 1e-6 relative.
            let chord = distance(previous.coordinate, s.coordinate).unwrap();
            assert!((chord / (nominal * 0.01) - 1.0).abs() < 1e-6, "{chord}");
            previous = s;
        }
        assert_eq!(m.kinematics().unwrap().acceleration_mps2, 0.0);
    }

    #[test]
    fn position_depends_only_on_elapsed_time_so_no_drift_accumulates() {
        // One model sampled 200 000 times, another jumping straight to the
        // same instants: identical, bit for bit.
        let mut dense = orbit(RotationDirection::Clockwise, 17.0);
        let mut sparse = orbit(RotationDirection::Clockwise, 17.0);
        sparse.sample_at(Timestamp::from_nanos(0)).unwrap();
        let mut last = None;
        for k in 0..200_000i64 {
            last = Some(
                dense
                    .sample_at(Timestamp::from_nanos(k * SEC / 50))
                    .unwrap(),
            );
            if k > 0 && k % 50_000 == 0 {
                let jump = sparse
                    .sample_at(Timestamp::from_nanos(k * SEC / 50))
                    .unwrap();
                assert_eq!(Some(jump), last);
            }
        }
        // After 4 000 s (33.3 revolutions) still exactly on the circle.
        let g = inverse(center(), last.unwrap().coordinate).unwrap();
        assert!((g.distance_m - 100.0).abs() < 1e-6);
    }

    #[test]
    fn time_must_strictly_increase() {
        let mut m = orbit(RotationDirection::Clockwise, 0.0);
        m.sample_at(t(5.0)).unwrap();
        for bad in [5.0, 4.0] {
            assert_eq!(
                m.sample_at(t(bad)),
                Err(MovementError::NonIncreasingTime {
                    previous: t(5.0),
                    current: t(bad)
                })
            );
        }
    }

    #[test]
    fn works_across_the_antimeridian_and_around_a_pole() {
        for c in [
            Coordinate::new(0.0, 180.0).unwrap(),
            Coordinate::new(-45.0, -179.9999).unwrap(),
            Coordinate::new(90.0, 0.0).unwrap(),
            Coordinate::new(89.9995, 100.0).unwrap(), // the orbit encloses the pole
        ] {
            let mut m = CircularModel::new(c, 0.0, 200.0, 6.0, RotationDirection::Clockwise, 0.0);
            for k in 0..120 {
                let s = m.sample_at(t(k as f64)).unwrap();
                assert!(
                    (distance(c, s.coordinate).unwrap() - 200.0).abs() < 1e-5,
                    "{c:?}"
                );
                assert!((0.0..360.0).contains(&s.course_deg.unwrap()));
            }
        }
    }
}
