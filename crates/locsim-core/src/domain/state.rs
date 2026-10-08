use super::error::InvalidTransition;

/// Lifecycle of a simulation. The only way to change state is
/// [`SimulationState::transition`], which rejects illegal moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimulationState {
    Idle,
    Starting,
    Running,
    Paused,
    Stopping,
    Error,
    Recovering,
}

impl SimulationState {
    pub const ALL: [SimulationState; 7] = [
        SimulationState::Idle,
        SimulationState::Starting,
        SimulationState::Running,
        SimulationState::Paused,
        SimulationState::Stopping,
        SimulationState::Error,
        SimulationState::Recovering,
    ];

    /// Legal transitions. Every non-idle state can reach `Stopping` so the
    /// simulation can always be shut down cleanly, and `Stopping` can only
    /// end in `Idle`.
    pub fn can_transition_to(self, next: SimulationState) -> bool {
        use SimulationState::*;
        matches!(
            (self, next),
            (Idle, Starting)
                | (Starting, Running | Error | Stopping)
                | (Running, Paused | Stopping | Error)
                | (Paused, Running | Stopping | Error)
                | (Stopping, Idle)
                | (Error, Recovering | Stopping)
                | (Recovering, Running | Error | Stopping)
        )
    }

    pub fn transition(self, next: SimulationState) -> Result<SimulationState, InvalidTransition> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(InvalidTransition {
                from: self,
                to: next,
            })
        }
    }

    /// Whether samples may be emitted in this state.
    pub fn emits_samples(self) -> bool {
        self == SimulationState::Running
    }
}

/// Externally reported health of the framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HealthState {
    Healthy,
    Degraded,
    Recovering,
    Failed,
    Stopped,
}

#[cfg(test)]
mod tests {
    use super::SimulationState::{self, *};

    #[test]
    fn transition_table_is_exactly_as_specified() {
        let allowed = [
            (Idle, Starting),
            (Starting, Running),
            (Starting, Error),
            (Starting, Stopping),
            (Running, Paused),
            (Running, Stopping),
            (Running, Error),
            (Paused, Running),
            (Paused, Stopping),
            (Paused, Error),
            (Stopping, Idle),
            (Error, Recovering),
            (Error, Stopping),
            (Recovering, Running),
            (Recovering, Error),
            (Recovering, Stopping),
        ];
        for from in SimulationState::ALL {
            for to in SimulationState::ALL {
                let expected = allowed.contains(&(from, to));
                assert_eq!(from.can_transition_to(to), expected, "{from:?} -> {to:?}");
                match from.transition(to) {
                    Ok(s) => assert!(expected && s == to),
                    Err(e) => assert!(!expected && e.from == from && e.to == to),
                }
            }
        }
    }

    #[test]
    fn happy_and_error_paths() {
        let mut s = Idle;
        for next in [Starting, Running, Paused, Running, Stopping, Idle] {
            s = s.transition(next).unwrap();
        }
        let mut s = Running;
        for next in [Error, Recovering, Running] {
            s = s.transition(next).unwrap();
        }
        assert!(s.emits_samples());
    }

    #[test]
    fn every_active_state_can_reach_idle() {
        for s in SimulationState::ALL {
            if s != Idle && s != Stopping {
                assert!(s.can_transition_to(Stopping), "{s:?}");
            }
        }
        assert!(Stopping.can_transition_to(Idle));
    }
}
