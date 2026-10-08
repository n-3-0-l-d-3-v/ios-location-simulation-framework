use super::{Kinematics, MovementError, MovementModel, MovementSample};
use crate::domain::{Boundary, Coordinate, MovementMode, Scenario, Timestamp, KINEMATIC_MARGIN};
use crate::geographic::{self, bearing_difference, normalize_bearing, wgs84};
use crate::rng::Rng;

/// Absolute headroom on the turn per step, on top of [`KINEMATIC_MARGIN`].
/// The validation gate reconstructs the turn from two bearings and a
/// meridian convergence, each good to ~1e-12°; this covers that with room.
const HEADING_MARGIN_DEG: f64 = 1e-9;
/// Steps shorter than this are below coordinate resolution (~1e-9 m) and are
/// treated as standing still.
const MIN_STEP_M: f64 = 1e-9;
const BISECTION_STEPS: usize = 60;
const MAX_CLAMP_ROUNDS: usize = 64;

// Independent random streams (see `NoiseEngine` for the same convention).
const STREAM_ROOT: u64 = 0x4D4F_5645; // "MOVE"
const STREAM_HEADING: u64 = 1;
const STREAM_SPEED: u64 = 2;
const STREAM_PAUSE: u64 = 3;

/// How the cruising speed is chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cruise {
    /// Always aim for this speed (bounded random walk).
    Constant(f64),
    /// Draw a new target uniformly from `[min, max]` after holding the
    /// current one for an exponentially distributed time (walking, driving).
    Varying {
        min_mps: f64,
        max_mps: f64,
        mean_hold_s: f64,
    },
}

/// Everything a [`SteeredModel`] needs; normally built from a scenario.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SteeredConfig {
    pub origin: Coordinate,
    pub altitude_m: f64,
    /// Effective speed limit (already including any displacement cap).
    pub max_speed_mps: f64,
    pub max_acceleration_mps2: f64,
    pub max_deceleration_mps2: f64,
    pub max_heading_rate_dps: f64,
    pub boundary: Option<Boundary>,
    pub cruise: Cruise,
    /// 0 = a completely new desired heading every step, 1 = never turns.
    pub heading_persistence: f64,
    /// Per-second probability of starting a pause.
    pub pause_probability: f64,
    pub max_pause_s: f64,
    pub seed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct State {
    t: Timestamp,
    position: Coordinate,
    speed_mps: f64,
    heading_deg: f64,
    acceleration_mps2: f64,
    heading_rate_dps: f64,
    cruise_mps: f64,
    pause_remaining_s: f64,
}

/// Position relative to the boundary, as seen from the mover.
#[derive(Debug, Clone, Copy)]
struct Fence {
    /// Distance to the boundary centre.
    centre_distance_m: f64,
    /// Bearing from the mover to the boundary centre.
    centre_bearing_deg: f64,
    /// Radius the model plans with (inside the real one).
    radius_m: f64,
}

impl Fence {
    /// Distance that can be travelled on `heading` before leaving the
    /// planning radius. A chord of the disc, computed in the plane; the
    /// planning radius is reduced to cover the curvature error and the
    /// result is re-checked with geodesics before it is used.
    fn room(&self, heading_deg: f64) -> f64 {
        let off = bearing_difference(heading_deg, self.centre_bearing_deg).to_radians();
        let along = self.centre_distance_m * off.cos();
        let across = self.centre_distance_m * off.sin();
        let half_chord = (self.radius_m * self.radius_m - across * across)
            .max(0.0)
            .sqrt();
        (along + half_chord).max(0.0)
    }
}

/// Modes B, C and D: a self-propelled mover with inertia.
///
/// # What is physically modelled
///
/// The state is position, speed and heading. Each step, over the elapsed
/// simulated time `dt`:
///
/// * **Speed** ramps linearly towards a target at no more than the
///   acceleration (or deceleration) limit, then holds; the distance covered
///   is the exact integral of that profile. A mover therefore starts from
///   rest, cannot jump to cruising speed, and cannot stop dead.
/// * **Heading** turns by at most `max_heading_rate × dt`, then the mover
///   travels along the geodesic leaving on that heading. The stored heading
///   is the bearing on arrival, so meridian convergence is carried correctly
///   at any latitude.
/// * **Boundary** (if any): the mover never plans to be closer to the edge
///   than its braking distance `v²/2d` on its current line, and steers
///   towards the centre once the edge is within braking distance plus two
///   turning radii. It can therefore always stop inside — it brakes for the
///   wall rather than being clipped at it — and it cannot wander away.
///
/// # What is merely configurable
///
/// *Behaviour* — which speed to aim for, when to pause, how the desired
/// heading wanders — is a simple seeded stochastic policy, not a model of
/// pedestrians or traffic: cruising speed is redrawn uniformly from the
/// configured range, pauses start with a fixed probability per second, and
/// the desired turn per step is uniform within `(1 − persistence) × 180°`.
/// Walking and driving differ only in their parameters. There is no road
/// network, no lateral-acceleration limit and no minimum turning radius.
/// `min_speed` bounds the cruising target; actual speed is lower while
/// accelerating, braking or paused.
///
/// # Time steps
///
/// One step is taken per sample, whatever its length. Limits hold for any
/// `dt`, but behavioural decisions are made once per step, so a trajectory
/// sampled at 10 Hz is not a refinement of the same seed sampled at 1 Hz.
#[derive(Debug, Clone)]
pub struct SteeredModel {
    config: SteeredConfig,
    state: Option<State>,
    rng_heading: Rng,
    rng_speed: Rng,
    rng_pause: Rng,
    boundary_clamps: u64,
}

impl SteeredModel {
    pub fn new(config: SteeredConfig) -> Self {
        let mut root = Rng::from_seed(config.seed).fork(STREAM_ROOT);
        let rng_heading = root.fork(STREAM_HEADING);
        let rng_speed = root.fork(STREAM_SPEED);
        let rng_pause = root.fork(STREAM_PAUSE);
        Self {
            config,
            state: None,
            rng_heading,
            rng_speed,
            rng_pause,
            boundary_clamps: 0,
        }
    }

    pub fn for_scenario(scenario: &Scenario) -> Result<Self, MovementError> {
        let m = &scenario.movement;
        let max_speed_mps = scenario.effective_max_speed_mps();
        let (cruise, pause_probability) = match scenario.mode {
            MovementMode::RandomWalk => {
                let step = m
                    .step_distance_m
                    .ok_or(MovementError::InvalidConfiguration(
                        "random walk needs a step distance",
                    ))?;
                if scenario.boundary().is_none() {
                    return Err(MovementError::InvalidConfiguration(
                        "random walk needs a radius",
                    ));
                }
                // A walk has no notion of stopping for a rest.
                (Cruise::Constant(step / scenario.update_interval_s), 0.0)
            }
            MovementMode::Walking | MovementMode::Driving => (
                Cruise::Varying {
                    min_mps: m.min_speed_mps,
                    max_mps: m.max_speed_mps.min(max_speed_mps),
                    mean_hold_s: m.speed_change_interval_s,
                },
                m.pause_probability,
            ),
            other => return Err(MovementError::UnsupportedMode(other)),
        };
        Ok(Self::new(SteeredConfig {
            origin: scenario.origin,
            altitude_m: scenario.altitude_m,
            max_speed_mps,
            max_acceleration_mps2: m.max_acceleration_mps2,
            max_deceleration_mps2: m.max_deceleration_mps2,
            max_heading_rate_dps: m.max_heading_rate_dps,
            boundary: scenario.boundary(),
            cruise,
            heading_persistence: m.heading_persistence,
            pause_probability,
            max_pause_s: m.max_pause_s,
            seed: scenario.seed,
        }))
    }

    pub fn config(&self) -> &SteeredConfig {
        &self.config
    }

    /// How often a planned step had to be shortened by the exact geodesic
    /// re-check. Expected to stay at zero; exposed for monitoring and tests.
    pub fn boundary_clamps(&self) -> u64 {
        self.boundary_clamps
    }

    fn initial_state(&mut self, t: Timestamp) -> State {
        let heading_deg = normalize_bearing(360.0 * self.rng_heading.next_f64());
        let draw = self.rng_speed.next_f64();
        let cruise_mps = match self.config.cruise {
            Cruise::Constant(v) => v,
            Cruise::Varying {
                min_mps, max_mps, ..
            } => min_mps + draw * (max_mps - min_mps),
        };
        // Always from rest: the first motion is an acceleration.
        State {
            t,
            position: self.config.origin,
            speed_mps: 0.0,
            heading_deg,
            acceleration_mps2: 0.0,
            heading_rate_dps: 0.0,
            cruise_mps,
            pause_remaining_s: 0.0,
        }
    }

    fn fence(&self, position: Coordinate) -> Result<Option<Fence>, MovementError> {
        let Some(b) = self.config.boundary else {
            return Ok(None);
        };
        let to_centre = geographic::inverse(position, b.center)?;
        // The chord in `Fence::room` is planar; on the ellipsoid it is off
        // by a relative (r/R)², which the planning radius gives up as well.
        let curvature = (b.radius_m / wgs84::MIN_CURVATURE_RADIUS).powi(2);
        Ok(Some(Fence {
            centre_distance_m: to_centre.distance_m,
            centre_bearing_deg: to_centre.initial_bearing_deg,
            radius_m: b.radius_m * (1.0 - KINEMATIC_MARGIN - curvature).max(0.0),
        }))
    }

    fn step(&mut self, st: State, t: Timestamp) -> Result<State, MovementError> {
        let c = self.config;
        let dt = t.seconds_since(st.t);
        let keep = 1.0 - KINEMATIC_MARGIN;
        let v_max = c.max_speed_mps * keep;
        let accel = c.max_acceleration_mps2 * keep;
        let decel = c.max_deceleration_mps2 * keep;
        let turn_cap = (c.max_heading_rate_dps * dt * keep - HEADING_MARGIN_DEG).clamp(0.0, 180.0);

        // A fixed number of draws per step keeps each stream aligned with
        // the sample index whatever happens.
        let u_pause = self.rng_pause.next_f64();
        let u_pause_length = self.rng_pause.next_f64();
        let u_speed_change = self.rng_speed.next_f64();
        let u_speed = self.rng_speed.next_f64();
        let u_turn = self.rng_heading.next_f64();

        // --- Behaviour: what the mover would like to do. -------------------
        let mut pause_remaining_s = (st.pause_remaining_s - dt).max(0.0);
        if st.pause_remaining_s <= 0.0 && c.pause_probability > 0.0 {
            let starts = 1.0 - (1.0 - c.pause_probability).powf(dt);
            if u_pause < starts {
                pause_remaining_s = u_pause_length * c.max_pause_s;
            }
        }
        let mut cruise_mps = st.cruise_mps;
        if let Cruise::Varying {
            min_mps,
            max_mps,
            mean_hold_s,
        } = c.cruise
        {
            if mean_hold_s > 0.0 && u_speed_change < 1.0 - (-dt / mean_hold_s).exp() {
                cruise_mps = min_mps + u_speed * (max_mps - min_mps);
            }
        }
        let target = if pause_remaining_s > 0.0 {
            0.0
        } else {
            cruise_mps.min(v_max)
        };
        let wander = (1.0 - c.heading_persistence) * (2.0 * u_turn - 1.0) * 180.0;

        // --- Steering. -----------------------------------------------------
        let fence = self.fence(st.position)?;
        let room = |heading: f64| fence.map_or(f64::INFINITY, |f| f.room(heading));
        let braking = |v: f64| {
            if decel > 0.0 {
                v * v / (2.0 * decel)
            } else {
                f64::INFINITY
            }
        };

        let desired_turn = match fence {
            None => wander,
            Some(f) => {
                let reference = st.speed_mps.max(target);
                let turn_rate = (c.max_heading_rate_dps * keep).to_radians();
                let turning_radius = if turn_rate > 0.0 {
                    reference / turn_rate
                } else {
                    0.0
                };
                let comfort = braking(reference) + 2.0 * turning_radius + reference * dt;
                if room(st.heading_deg) < comfort {
                    bearing_difference(st.heading_deg, f.centre_bearing_deg)
                } else {
                    wander
                }
            }
        };
        let mut turn = desired_turn.clamp(-turn_cap, turn_cap);
        let mut depart = normalize_bearing(st.heading_deg + turn);
        let mut available = room(depart);
        if braking(st.speed_mps) > available {
            // Turning would point the mover somewhere it can no longer stop
            // short of. Straight on is always safe: the previous step left
            // at least a braking distance on this line.
            turn = 0.0;
            depart = st.heading_deg;
            available = room(depart);
        }

        // --- Speed: ramp towards the target, never out-running the brakes. --
        let v0 = st.speed_mps;
        let profile = |goal: f64| -> (f64, f64) {
            let rate = if goal >= v0 { accel } else { decel };
            let change = (goal - v0).abs();
            if rate <= 0.0 || change == 0.0 {
                return (v0 * dt, v0);
            }
            let ramp = change / rate;
            if ramp >= dt {
                let v1 = if goal >= v0 {
                    v0 + rate * dt
                } else {
                    v0 - rate * dt
                };
                (0.5 * (v0 + v1) * dt, v1)
            } else {
                (0.5 * (v0 + goal) * ramp + goal * (dt - ramp), goal)
            }
        };
        let stoppable = |goal: f64| {
            let (distance, v1) = profile(goal);
            distance <= available && braking(v1) <= available - distance
        };
        let mut goal = target;
        if !stoppable(goal) {
            // Distance and end speed both grow with the goal, so the
            // feasible goals form an interval starting at zero.
            let (mut low, mut high) = (0.0, goal);
            for _ in 0..BISECTION_STEPS {
                let mid = 0.5 * (low + high);
                if stoppable(mid) {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            goal = low;
        }
        let (planned, mut speed_mps) = profile(goal);
        // The gate compares the speed change with `limit × dt` in its own
        // arithmetic; if rounding at a tiny `dt` would exceed that, hold.
        let change = speed_mps - v0;
        if change > c.max_acceleration_mps2 * dt || -change > c.max_deceleration_mps2 * dt {
            speed_mps = v0;
        }

        // --- Move along the geodesic, re-checking the hard limits exactly. --
        let mut distance = planned.min(available);
        let mut position = st.position;
        let mut arrival = depart;
        let mut clamped = false;
        for _ in 0..MAX_CLAMP_ROUNDS {
            if distance < MIN_STEP_M {
                position = st.position;
                arrival = depart;
                break;
            }
            let (p, a) = geographic::direct(st.position, depart, distance)?;
            let inside = match c.boundary {
                Some(b) => geographic::distance(b.center, p)? <= b.radius_m,
                None => true,
            };
            if inside && geographic::distance(st.position, p)? <= c.max_speed_mps * dt {
                position = p;
                arrival = a;
                break;
            }
            clamped = true;
            distance *= 0.5;
        }
        if clamped {
            self.boundary_clamps += 1;
        }

        Ok(State {
            t,
            position,
            speed_mps,
            heading_deg: arrival,
            acceleration_mps2: change_rate(speed_mps - v0, dt),
            heading_rate_dps: change_rate(turn, dt),
            cruise_mps,
            pause_remaining_s,
        })
    }
}

fn change_rate(change: f64, dt: f64) -> f64 {
    if dt > 0.0 {
        change / dt
    } else {
        0.0
    }
}

impl MovementModel for SteeredModel {
    fn sample_at(&mut self, t: Timestamp) -> Result<MovementSample, MovementError> {
        let state = match self.state {
            None => self.initial_state(t),
            Some(previous) => {
                if t <= previous.t {
                    return Err(MovementError::NonIncreasingTime {
                        previous: previous.t,
                        current: t,
                    });
                }
                self.step(previous, t)?
            }
        };
        self.state = Some(state);
        Ok(MovementSample {
            coordinate: state.position,
            altitude_m: self.config.altitude_m,
            speed_mps: Some(state.speed_mps),
            // A course is a direction of travel; a stopped mover has none.
            course_deg: (state.speed_mps > 0.0).then_some(state.heading_deg),
        })
    }

    fn kinematics(&self) -> Option<Kinematics> {
        self.state.map(|s| Kinematics {
            position: s.position,
            speed_mps: s.speed_mps,
            heading_deg: s.heading_deg,
            acceleration_mps2: s.acceleration_mps2,
            heading_rate_dps: s.heading_rate_dps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geographic::distance;

    const SEC: i64 = 1_000_000_000;

    fn t(seconds: f64) -> Timestamp {
        Timestamp::from_nanos((seconds * 1e9).round() as i64)
    }

    fn origin() -> Coordinate {
        Coordinate::new(12.9352, 77.6245).unwrap()
    }

    fn walker(seed: u64) -> SteeredConfig {
        SteeredConfig {
            origin: origin(),
            altitude_m: 920.0,
            max_speed_mps: 1.8,
            max_acceleration_mps2: 0.8,
            max_deceleration_mps2: 1.2,
            max_heading_rate_dps: 60.0,
            boundary: None,
            cruise: Cruise::Varying {
                min_mps: 0.8,
                max_mps: 1.8,
                mean_hold_s: 30.0,
            },
            heading_persistence: 0.9,
            pause_probability: 0.0,
            max_pause_s: 0.0,
            seed,
        }
    }

    fn run(config: SteeredConfig, samples: usize, dt_s: f64) -> (Vec<Kinematics>, SteeredModel) {
        let mut m = SteeredModel::new(config);
        let out = (0..samples)
            .map(|k| {
                m.sample_at(t(k as f64 * dt_s)).unwrap();
                m.kinematics().unwrap()
            })
            .collect();
        (out, m)
    }

    #[test]
    fn starts_at_rest_at_the_origin_and_accelerates_gradually() {
        let mut m = SteeredModel::new(walker(1));
        let first = m.sample_at(t(0.0)).unwrap();
        assert_eq!(first.coordinate, origin());
        assert_eq!((first.speed_mps, first.course_deg), (Some(0.0), None));
        assert_eq!(first.altitude_m, 920.0);

        // 0.8 m/s² at 10 Hz: 0.08 m/s per sample, so cruising speed (≥ 0.8)
        // cannot be reached in fewer than 10 samples.
        let mut previous = 0.0;
        for k in 1..=9 {
            let s = m.sample_at(t(k as f64 * 0.1)).unwrap();
            let v = s.speed_mps.unwrap();
            assert!(v > previous && v - previous <= 0.08, "sample {k}: {v}");
            assert!(v < 0.8);
            assert!(s.course_deg.is_some());
            previous = v;
        }
    }

    #[test]
    fn reaches_and_holds_a_cruising_speed_within_the_range() {
        let (states, _) = run(
            SteeredConfig {
                cruise: Cruise::Varying {
                    min_mps: 0.8,
                    max_mps: 1.8,
                    mean_hold_s: 0.0, // never changes
                },
                ..walker(4)
            },
            200,
            0.5,
        );
        let cruising = states[199].speed_mps;
        assert!((0.8..=1.8).contains(&cruising), "{cruising}");
        assert!(states[20..].iter().all(|k| k.speed_mps == cruising));
        assert!(states[20..].iter().all(|k| k.acceleration_mps2 == 0.0));
    }

    #[test]
    fn distance_covered_is_the_integral_of_the_speed_profile() {
        // No turning, constant target: after the ramp the mover has covered
        // v²/2a during it and v per second after it.
        let config = SteeredConfig {
            cruise: Cruise::Constant(1.6),
            heading_persistence: 1.0,
            ..walker(2)
        };
        let (states, _) = run(config, 101, 0.25); // 25 s
        let v = states[100].speed_mps;
        assert!((v - 1.6).abs() < 1e-5);
        let accel = 0.8 * (1.0 - KINEMATIC_MARGIN);
        let ramp_time = v / accel;
        let expected = 0.5 * v * ramp_time + v * (25.0 - ramp_time);
        let covered = distance(origin(), states[100].position).unwrap();
        assert!((covered - expected).abs() < 1e-6, "{covered} vs {expected}");
    }

    #[test]
    fn same_distance_whatever_the_sampling_rate_when_going_straight() {
        let config = SteeredConfig {
            cruise: Cruise::Constant(1.6),
            heading_persistence: 1.0,
            ..walker(2)
        };
        let coarse = run(config, 13, 5.0).0[12].position; // 60 s
        let fine = run(config, 6_001, 0.01).0[6_000].position;
        // Identical profile, integrated exactly in both cases; what remains
        // is rounding accumulated over 6 000 geodesic steps.
        assert!(distance(coarse, fine).unwrap() < 1e-5);
    }

    #[test]
    fn pauses_brake_to_a_stop_and_resume_from_rest() {
        let config = SteeredConfig {
            pause_probability: 0.05,
            max_pause_s: 15.0,
            ..walker(9)
        };
        let (states, _) = run(config, 4_000, 0.5);
        let stopped = states.iter().filter(|k| k.speed_mps == 0.0).count();
        assert!(
            stopped > 100 && stopped < 3_000,
            "{stopped} stopped samples"
        );
        for w in states.windows(2) {
            let dv = w[1].speed_mps - w[0].speed_mps;
            assert!(dv <= 0.8 * 0.5 && -dv <= 1.2 * 0.5, "dv {dv}");
        }
        // Position holds exactly while stopped.
        for w in states.windows(2) {
            if w[0].speed_mps == 0.0 && w[1].speed_mps == 0.0 {
                assert_eq!(w[0].position, w[1].position);
            }
        }
    }

    #[test]
    fn heading_persistence_one_travels_a_geodesic() {
        let config = SteeredConfig {
            heading_persistence: 1.0,
            ..walker(3)
        };
        let (states, _) = run(config, 600, 1.0);
        assert!(states.iter().all(|k| k.heading_rate_dps == 0.0));
        // The last point lies on the geodesic through the first two.
        let start = states[1].position;
        let line = geographic::inverse(start, states[2].position).unwrap();
        let total = distance(start, states[599].position).unwrap();
        let predicted = geographic::destination(start, line.initial_bearing_deg, total).unwrap();
        assert!(distance(predicted, states[599].position).unwrap() < 1e-3);
    }

    #[test]
    fn lower_persistence_turns_more() {
        let turning = |persistence: f64| -> f64 {
            let config = SteeredConfig {
                heading_persistence: persistence,
                ..walker(5)
            };
            let (states, _) = run(config, 2_000, 1.0);
            states.iter().map(|k| k.heading_rate_dps.abs()).sum::<f64>() / 2_000.0
        };
        let (steady, erratic) = (turning(0.95), turning(0.2));
        assert!(steady > 0.0);
        assert!(erratic > 5.0 * steady, "{erratic} vs {steady}");
        assert!(erratic <= 60.0);
    }

    #[test]
    fn bounded_mover_brakes_for_the_edge_and_never_needs_clamping() {
        let boundary = Boundary {
            center: origin(),
            radius_m: 40.0,
        };
        let config = SteeredConfig {
            boundary: Some(boundary),
            cruise: Cruise::Constant(1.5),
            heading_persistence: 0.8,
            ..walker(11)
        };
        let (states, model) = run(config, 20_000, 1.0);
        let mut furthest = 0.0f64;
        let mut inner = false;
        for k in &states {
            let d = distance(origin(), k.position).unwrap();
            assert!(d <= 40.0, "{d}");
            furthest = furthest.max(d);
            inner |= d < 10.0;
        }
        // It uses the area rather than hugging the centre or the edge, and
        // the exact re-check never had to shorten a planned step.
        assert!(furthest > 30.0 && inner, "furthest {furthest}");
        assert_eq!(model.boundary_clamps(), 0);
        // It keeps moving: no deadlock against the fence.
        let moving = states.iter().filter(|k| k.speed_mps > 0.5).count();
        assert!(moving > 15_000, "{moving}");
    }

    #[test]
    fn time_must_strictly_increase() {
        let mut m = SteeredModel::new(walker(1));
        m.sample_at(t(3.0)).unwrap();
        for bad in [3.0, 2.0] {
            assert_eq!(
                m.sample_at(t(bad)),
                Err(MovementError::NonIncreasingTime {
                    previous: t(3.0),
                    current: t(bad)
                })
            );
        }
        m.sample_at(Timestamp::from_nanos(3 * SEC + 1)).unwrap();
    }

    #[test]
    fn same_seed_same_trajectory_different_seed_different_trajectory() {
        let a = run(walker(42), 500, 1.0).0;
        let b = run(walker(42), 500, 1.0).0;
        let c = run(walker(43), 500, 1.0).0;
        assert_eq!(a, b);
        // Not just different in the last bit: hundreds of metres apart.
        let apart = distance(a[499].position, c[499].position).unwrap();
        assert!(apart > 50.0, "{apart}");
    }

    #[test]
    fn velocity_components_follow_heading() {
        let k = Kinematics {
            position: origin(),
            speed_mps: 2.0,
            heading_deg: 90.0,
            acceleration_mps2: 0.0,
            heading_rate_dps: 0.0,
        };
        let (east, north) = k.velocity_en_mps();
        assert!((east - 2.0).abs() < 1e-12 && north.abs() < 1e-12);
    }
}
