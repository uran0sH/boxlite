//! Storage operations (volume mounting).
//!
//! Provides unified abstraction for mounting different volume types:
//! - Virtiofs: Host-shared directories via virtio-fs
//! - Block devices: Disk images attached via virtio-blk

mod block_device;
mod copy;
mod virtiofs;
mod volume;

pub use copy::copy_layers_to_disk;
pub use volume::mount_volumes;
