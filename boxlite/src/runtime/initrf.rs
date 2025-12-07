//! Resolved rootfs types and metadata.

use std::path::{Path, PathBuf};

use boxlite_shared::errors::{BoxliteError, BoxliteResult};

/// A fully resolved and ready-to-use rootfs.
///
/// This struct represents the complete result of rootfs preparation:
/// - Image pulled (if needed)
/// - Layers extracted/overlayed
/// - Guest binary injected and validated
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InitRootfs {
    /// Path to the merged/final rootfs directory
    pub path: PathBuf,

    /// How this rootfs was prepared
    #[serde(skip)]
    pub strategy: Strategy,

    /// Kernel images path (for Firecracker/microVM)
    pub kernel: Option<PathBuf>,

    /// Initrd path (optional)
    pub initrd: Option<PathBuf>,

    /// Environment variables from the init image config (e.g., PATH)
    #[serde(default)]
    pub env: Vec<(String, String)>,
}

/// Strategy used to prepare the rootfs.
///
/// This tracks how the rootfs was assembled, which is important for:
/// - Cleanup logic (overlayfs mounts need unmounting)
/// - Debugging (understand which strategy was used)
/// - Performance metrics (compare overlay vs extraction)
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Strategy {
    /// Direct path provided by user (no processing needed)
    #[default]
    Direct,

    /// Layers extracted into a single directory
    ///
    /// Used on macOS (no overlayfs) and as fallback on Linux
    Extracted {
        /// Number of layers extracted
        layers: usize,
    },

    /// Linux overlayfs mount (requires cleanup on drop)
    ///
    /// This is the preferred strategy on Linux when CAP_SYS_ADMIN is available
    OverlayMount {
        /// Lower directories (read-only layers)
        lower: Vec<PathBuf>,
        /// Upper directory (writable layer)
        upper: PathBuf,
        /// Work directory (required by overlayfs)
        work: PathBuf,
    },
}

impl InitRootfs {
    /// Create a new InitRootfs, injecting the guest binary if needed.
    pub fn new(
        path: PathBuf,
        strategy: Strategy,
        kernel: Option<PathBuf>,
        initrd: Option<PathBuf>,
        env: Vec<(String, String)>,
    ) -> BoxliteResult<Self> {
        // Inject guest binary if not already present
        Self::inject_guest_binary_if_needed(&path)?;

        Ok(Self {
            path,
            strategy,
            kernel,
            initrd,
            env,
        })
    }

    /// Inject guest binary into init rootfs if not already present.
    ///
    /// Copies boxlite-guest into /boxlite/bin/ so it can be executed
    /// directly without needing a virtiofs mount at boot time.
    /// This avoids the "must be superuser to use mount" error on Debian
    /// where util-linux mount has stricter permission checks than BusyBox.
    fn inject_guest_binary_if_needed(rootfs_path: &Path) -> BoxliteResult<()> {
        let dest_dir = rootfs_path.join("boxlite/bin");
        let dest_path = dest_dir.join("boxlite-guest");

        let guest_bin = crate::util::find_binary("boxlite-guest")?;

        // PERF: Check if binary needs update using fast mtime+size comparison first.
        // MD5 computation reads the entire file (~10MB) which takes ~47ms.
        // mtime+size comparison uses only file metadata (~0.1ms).
        // We only fall back to MD5 if metadata suggests a potential change.
        if dest_path.exists() {
            if Self::is_binary_up_to_date(&guest_bin, &dest_path)? {
                return Ok(());
            }

            // Remove old binary before copying (it might be read-only 0o555)
            std::fs::remove_file(&dest_path).map_err(|e| {
                BoxliteError::Storage(format!(
                    "Failed to remove old guest binary {}: {}",
                    dest_path.display(),
                    e
                ))
            })?;
        }

        std::fs::create_dir_all(&dest_dir).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to create guest bin directory {}: {}",
                dest_dir.display(),
                e
            ))
        })?;

        std::fs::copy(&guest_bin, &dest_path).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to copy guest binary to {}: {}",
                dest_path.display(),
                e
            ))
        })?;

        // Ensure executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest_path, std::fs::Permissions::from_mode(0o555)).map_err(
                |e| {
                    BoxliteError::Storage(format!(
                        "Failed to set permissions on {}: {}",
                        dest_path.display(),
                        e
                    ))
                },
            )?;
        }

        tracing::info!(
            "✅ Guest binary updated successfully at {}",
            dest_path.display()
        );
        Ok(())
    }

    /// Check if destination binary is up-to-date compared to source.
    ///
    /// PERF: Uses fast mtime+size comparison instead of MD5 checksums.
    /// This reduces check time from ~47ms (reading entire file) to ~0.1ms (metadata only).
    ///
    /// The comparison logic:
    /// 1. If file sizes differ → definitely needs update
    /// 2. If dest mtime >= source mtime AND sizes match → up-to-date
    /// 3. Otherwise → needs update (source is newer or was modified)
    ///
    /// Note: mtime comparison is safe here because:
    /// - Source binary changes only during development or package updates
    /// - We copy with std::fs::copy which preserves content, and dest mtime
    ///   will be set to copy time (always >= source mtime after successful copy)
    fn is_binary_up_to_date(source: &Path, dest: &Path) -> BoxliteResult<bool> {
        let source_meta = std::fs::metadata(source).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to get metadata for {}: {}",
                source.display(),
                e
            ))
        })?;

        let dest_meta = std::fs::metadata(dest).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to get metadata for {}: {}",
                dest.display(),
                e
            ))
        })?;

        let source_size = source_meta.len();
        let dest_size = dest_meta.len();

        // Quick rejection: different sizes means definitely different content
        if source_size != dest_size {
            tracing::info!(
                "Guest binary size changed ({} -> {} bytes), updating...",
                dest_size,
                source_size
            );
            return Ok(false);
        }

        // Compare modification times
        // If dest was modified after source, it's up-to-date (we copied it)
        let source_mtime = source_meta.modified().map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to get mtime for {}: {}",
                source.display(),
                e
            ))
        })?;

        let dest_mtime = dest_meta.modified().map_err(|e| {
            BoxliteError::Storage(format!("Failed to get mtime for {}: {}", dest.display(), e))
        })?;

        if dest_mtime >= source_mtime {
            tracing::debug!(
                "Guest binary at {} is up-to-date (size: {} bytes)",
                dest.display(),
                dest_size
            );
            return Ok(true);
        }

        // Source is newer than dest - needs update
        tracing::info!("Guest binary source is newer than destination, updating...");
        Ok(false)
    }

    /// Clean up this rootfs.
    ///
    /// Behavior depends on strategy:
    /// - `Direct`: No-op (user-provided path, don't delete)
    /// - `Extracted`: Remove the directory
    /// - `OverlayMount`: Unmount, then remove the directory
    ///
    /// Returns Ok(()) if cleanup succeeded or wasn't needed.
    pub fn cleanup(&self) -> BoxliteResult<()> {
        match &self.strategy {
            Strategy::Direct => {
                // User-provided path - don't clean up
                tracing::debug!(
                    "Skipping cleanup for direct rootfs: {}",
                    self.path.display()
                );
                Ok(())
            }
            Strategy::Extracted { layers } => {
                tracing::info!(
                    "Cleaning up extracted rootfs ({} layers): {}",
                    layers,
                    self.path.display()
                );
                // Remove parent directory (contains merged/)
                if let Some(parent) = self.path.parent() {
                    Self::remove_directory(parent)
                } else {
                    Self::remove_directory(&self.path)
                }
            }
            Strategy::OverlayMount { .. } => {
                tracing::info!("Cleaning up overlay mount: {}", self.path.display());

                #[cfg(target_os = "linux")]
                {
                    // Unmount overlay first
                    Self::unmount_overlay(&self.path)?;
                }

                // Remove parent directory (contains merged/, upper/, work/, patch/)
                if let Some(parent) = self.path.parent() {
                    Self::remove_directory(parent)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Unmount overlayfs (Linux only)
    #[cfg(target_os = "linux")]
    fn unmount_overlay(merged_dir: &Path) -> BoxliteResult<()> {
        if !merged_dir.exists() {
            return Ok(());
        }

        match std::process::Command::new("umount")
            .arg(merged_dir)
            .status()
        {
            Ok(status) if status.success() => {
                tracing::debug!("Unmounted overlay: {}", merged_dir.display());
                Ok(())
            }
            Ok(status) => {
                tracing::warn!(
                    "Failed to unmount overlay {}: exit status {}",
                    merged_dir.display(),
                    status
                );
                Err(BoxliteError::Storage(format!(
                    "umount failed with status {}",
                    status
                )))
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to execute umount for {}: {}",
                    merged_dir.display(),
                    e
                );
                Err(BoxliteError::Storage(format!(
                    "umount execution failed: {}",
                    e
                )))
            }
        }
    }

    /// Remove directory recursively
    fn remove_directory(path: &Path) -> BoxliteResult<()> {
        if let Err(e) = std::fs::remove_dir_all(path) {
            tracing::warn!(
                "Failed to cleanup rootfs directory {}: {}",
                path.display(),
                e
            );
            Err(BoxliteError::Storage(format!("cleanup failed: {}", e)))
        } else {
            tracing::info!("Cleaned up rootfs directory: {}", path.display());
            Ok(())
        }
    }
}
