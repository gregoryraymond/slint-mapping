//! Concrete [`Router`](crate::Router) implementations.
//!
//! Each adapter lives in its own submodule and is re-exported at this
//! module's root. Adding a new routing engine = drop a file in this
//! directory, register it here. Mirrors the layout of
//! [`crate::sources`] for tile sources.

#[cfg(feature = "routing")]
mod osrm;
#[cfg(feature = "routing")]
pub use osrm::{OsrmRouter, OSRM_DEMO_URL};
