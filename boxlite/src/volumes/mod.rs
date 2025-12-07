//! Storage operations (disk image management).
//!
//! Provides disk image creation and management for Box block devices.

mod disk;

pub use disk::{Disk, DiskManager};
