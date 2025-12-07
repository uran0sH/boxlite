//! Virtiofs mount helper.
//!
//! Mounts virtiofs filesystems shared from the host.

use std::path::Path;

use boxlite_shared::errors::{BoxliteError, BoxliteResult};
use nix::mount::{mount, MsFlags};

/// Mounts virtiofs filesystems.
pub struct VirtiofsMount;

impl VirtiofsMount {
    /// Mount virtiofs tag to mount point.
    pub fn mount(tag: &str, mount_point: &Path) -> BoxliteResult<()> {
        tracing::info!("Mounting virtiofs: {} → {}", tag, mount_point.display());

        // Create mount point
        std::fs::create_dir_all(mount_point).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to create mount point {}: {}",
                mount_point.display(),
                e
            ))
        })?;

        // Mount using nix
        mount(
            Some(tag),
            mount_point,
            Some("virtiofs"),
            MsFlags::empty(),
            None::<&str>,
        )
        .map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to mount virtiofs {} to {}: {}",
                tag,
                mount_point.display(),
                e
            ))
        })?;

        tracing::info!("Mounted virtiofs: {} → {}", tag, mount_point.display());
        Ok(())
    }
}
