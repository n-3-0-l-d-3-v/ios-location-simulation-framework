use super::{Kinematics, MovementError, MovementModel, MovementSample};
use crate::domain::{Scenario, Timestamp};
use crate::route::{self, RoutePlan, RouteState};

/// Mode F: replay of a recorded route.
///
/// To the rest of the pipeline this is one more movement model: it turns
/// simulated time into a noise-free sample, and that sample goes through
/// noise and the final validation gate like any other. The model itself only
/// evaluates an admitted [`RoutePlan`] at `t − t₀`, where `t₀` is the time of
/// the first sample; it keeps no integration state, so the same instant
/// always yields the same sample, whatever was sampled before.
#[derive(Debug, Clone)]
pub struct RouteModel {
    plan: RoutePlan,
    start: Option<Timestamp>,
    last: Option<(Timestamp, RouteState)>,
}

impl RouteModel {
    /// Wraps a plan. The caller is responsible for having admitted it; use
    /// [`RouteModel::for_scenario`] to build and admit in one step.
    pub fn new(plan: RoutePlan) -> Self {
        Self {
            plan,
            start: None,
            last: None,
        }
    }

    /// Admits the scenario's route against the scenario's movement limits
    /// and wraps the resulting trajectory. A route that breaks a limit is an
    /// error here, before any sample exists.
    pub fn for_scenario(scenario: &Scenario) -> Result<Self, MovementError> {
        route::admit(scenario)
            .map(Self::new)
            .map_err(MovementError::RouteRejected)
    }

    pub fn plan(&self) -> &RoutePlan {
        &self.plan
    }
}

impl MovementModel for RouteModel {
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
        let elapsed_ns = t.as_nanos().saturating_sub(start.as_nanos());
        let state = self.plan.state_at(elapsed_ns)?;
        self.last = Some((t, state));
        Ok(MovementSample {
            coordinate: state.coordinate,
            altitude_m: state.altitude_m,
            speed_mps: Some(state.speed_mps),
            course_deg: state.course_deg,
        })
    }

    fn kinematics(&self) -> Option<Kinematics> {
        self.last.map(|(_, s)| Kinematics {
            position: s.coordinate,
            speed_mps: s.speed_mps,
            // At rest a route has no direction; north is a placeholder.
            heading_deg: s.course_deg.unwrap_or(0.0),
            acceleration_mps2: s.acceleration_mps2,
            heading_rate_dps: s.heading_rate_dps,
        })
    }

    fn is_complete(&self) -> bool {
        self.last.is_some_and(|(_, s)| s.complete)
    }
}
