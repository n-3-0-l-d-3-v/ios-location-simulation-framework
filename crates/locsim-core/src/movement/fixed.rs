use super::{MovementError, MovementModel, MovementSample};
use crate::domain::{Coordinate, Timestamp};

/// Mode A: a stationary position. Speed is exactly zero and there is no
/// course, because a stationary fix has no direction of travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedModel {
    coordinate: Coordinate,
    altitude_m: f64,
}

impl FixedModel {
    pub fn new(coordinate: Coordinate, altitude_m: f64) -> Self {
        Self {
            coordinate,
            altitude_m,
        }
    }
}

impl MovementModel for FixedModel {
    fn sample_at(&mut self, _t: Timestamp) -> Result<MovementSample, MovementError> {
        Ok(MovementSample {
            coordinate: self.coordinate,
            altitude_m: self.altitude_m,
            speed_mps: Some(0.0),
            course_deg: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_configured_position_at_any_time() {
        let origin = Coordinate::new(12.9352, 77.6245).unwrap();
        let mut m = FixedModel::new(origin, 920.0);
        for nanos in [i64::MIN, 0, 1, 1_700_000_000_000_000_000, i64::MAX] {
            let s = m.sample_at(Timestamp::from_nanos(nanos)).unwrap();
            assert_eq!(s.coordinate, origin);
            assert_eq!(s.altitude_m, 920.0);
            assert_eq!(s.speed_mps, Some(0.0));
            assert_eq!(s.course_deg, None);
        }
    }
}
