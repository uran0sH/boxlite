//! Box lifecycle management.
//!
//! This module provides functionality for tracking and managing the lifecycle
//! of Box instances.
//!
//! # Overview
//!
//! - **BoxManager**: Thread-safe registry for tracking live boxes
//! - **BoxId**: Unique identifier (ULID format) for each box
//! - **BoxState**: Lifecycle states (Starting, Running, Stopped, Failed)
//! - **BoxInfo**: Public metadata about a box (for list operations)
//!
//! # Example
//!
//! ```rust,no_run
//! use boxlite_runtime::{BoxliteRuntime, management::BoxState};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let runtime = BoxliteRuntime::new(BoxliteOptions::default())?;
//!
//! // Create boxes
//! let (id1, box1) = runtime.create(Default::default())?;
//! let (id2, box2) = runtime.create(Default::default())?;
//!
//! // List all boxes
//! for info in runtime.list()? {
//!     println!("{}: {:?} (PID {})", info.id, info.state, info.pid.unwrap_or(0));
//! }
//! # Ok(())
//! # }
//! ```

mod manager;

pub use crate::runtime::types::{BoxID, BoxInfo, BoxState, generate_box_id};
pub use manager::BoxManager;

// Re-export BoxMetadata only within crate for use in runtime.rs
pub(crate) use crate::runtime::types::BoxMetadata;
