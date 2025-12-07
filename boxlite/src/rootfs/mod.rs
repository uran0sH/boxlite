//! Rootfs management
//!
//! This module handles rootfs preparation and management for boxes.

mod builder;
mod copy_mount;
mod dns;
pub(crate) mod operations;

pub use builder::{PreparedRootfs, RootfsBuilder};
pub use copy_mount::{CopyMode, CopyMountOptions, copy_based_mount};
pub use dns::configure_container_dns;
