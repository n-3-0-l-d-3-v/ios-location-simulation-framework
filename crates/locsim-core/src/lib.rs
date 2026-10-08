//! Platform-independent core of the synthetic location framework.
//!
//! Nothing in this crate depends on iOS or Core Location. Layers are exposed
//! as modules; lower layers never import higher ones:
//!
//! `rng` → `geographic` → `domain` → (movement, noise, scenario, … in later tickets)

#![forbid(unsafe_code)]

pub mod domain;
pub mod geographic;
pub mod rng;
pub mod scheduler;
pub mod validation;
