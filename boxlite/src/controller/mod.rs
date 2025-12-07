//! Subprocess-based Box controller management.
//!
//! This module provides the `ShimController` which manages Box lifecycle
//! by spawning `boxlite-shim` in a subprocess. The subprocess isolation
//! ensures that process takeover doesn't affect the host application.

mod log_stream;
mod shim;
mod spawn;

pub use shim::ShimController;
