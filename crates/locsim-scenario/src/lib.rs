//! Scenario documents: strict JSON import and export of
//! [`locsim_core::domain::Scenario`], with schema versioning.
//!
//! This crate turns text into a validated `Scenario` and back. It reads and
//! writes strings only; files and persistence are a separate concern.

#![forbid(unsafe_code)]
// The document layer is not yet used by a public entry point.
#![allow(dead_code)]

mod error;
mod json;
mod schema;

pub use error::ScenarioError;
