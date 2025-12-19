//! OCI images object with encapsulated operations.
//!
//! This module provides `ImageObject`, a self-contained handle to a pulled
//! OCI image that encapsulates all image-related operations (config loading,
//! layer access, inspection).

use std::path::PathBuf;

use super::manager::ImageManifest;
use crate::images::store::SharedImageStore;
use boxlite_shared::errors::{BoxliteError, BoxliteResult};

// ============================================================================
// IMAGE OBJECT
// ============================================================================

/// A pulled OCI image with all associated operations.
///
/// This object represents a complete pulled image and provides access to:
/// - Image metadata (reference, layers, config)
/// - Container configuration
/// - Layer file paths
/// - Inspection operations
///
/// Created by `ImageManager::pull()`.
///
/// Thread Safety: Holds `Arc<ImageStore>` which handles locking internally.
#[derive(Clone)]
pub struct ImageObject {
    /// Image reference (e.g., "python:alpine")
    reference: String,

    /// Manifest with layer information
    manifest: ImageManifest,

    /// Shared reference to store for layer/config access
    store: SharedImageStore,
}

impl ImageObject {
    /// Create new ImageObject (internal use only)
    pub(super) fn new(reference: String, manifest: ImageManifest, store: SharedImageStore) -> Self {
        Self {
            reference,
            manifest,
            store,
        }
    }

    // ========================================================================
    // METADATA OPERATIONS
    // ========================================================================

    /// Get the image reference (e.g., "python:alpine")
    #[allow(dead_code)]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Get list of layer digests
    #[allow(dead_code)]
    pub fn layer_digests(&self) -> Vec<&str> {
        self.manifest
            .layers
            .iter()
            .map(|l| l.digest.as_str())
            .collect()
    }

    /// Get config digest
    #[allow(dead_code)]
    pub fn config_digest(&self) -> &str {
        &self.manifest.config_digest
    }

    /// Get number of layers
    #[allow(dead_code)]
    pub fn layer_count(&self) -> usize {
        self.manifest.layers.len()
    }

    // ========================================================================
    // CONFIG OPERATIONS
    // ========================================================================

    /// Load original OCI image configuration
    ///
    /// Returns the complete OCI ImageConfiguration structure as defined in the
    /// OCI image spec. This includes all fields from the image config.json.
    ///
    /// Use `ContainerConfig::from_oci_config()` if you need extracted container
    /// runtime configuration (entrypoint, env, workdir).
    pub async fn load_config(&self) -> BoxliteResult<oci_spec::image::ImageConfiguration> {
        let config_json = self.store.config(&self.manifest.config_digest).await?;

        serde_json::from_str(&config_json)
            .map_err(|e| BoxliteError::Storage(format!("Failed to parse image config: {}", e)))
    }

    // ========================================================================
    // LAYER OPERATIONS
    // ========================================================================

    /// Get path to a specific layer tarball
    ///
    /// Layers are indexed from 0 (base layer) to N-1 (top layer).
    #[allow(dead_code)]
    pub async fn layer_tarball(&self, layer_index: usize) -> BoxliteResult<PathBuf> {
        let layer = self.manifest.layers.get(layer_index).ok_or_else(|| {
            BoxliteError::Storage(format!(
                "Layer index {} out of bounds (total layers: {})",
                layer_index,
                self.manifest.layers.len()
            ))
        })?;

        Ok(self.store.layer_tarball(&layer.digest).await)
    }

    /// Get paths to all layer tarballs (ordered bottom to top)
    pub async fn layer_tarballs(&self) -> Vec<PathBuf> {
        let mut paths = Vec::with_capacity(self.manifest.layers.len());
        for layer in &self.manifest.layers {
            paths.push(self.store.layer_tarball(&layer.digest).await);
        }
        paths
    }

    /// Get paths to extracted layer directories (with caching)
    ///
    /// This method extracts each layer tarball to a separate directory and caches
    /// the result. Subsequent calls return the cached extracted directories.
    ///
    /// Uses rayon for parallel extraction of multiple layers.
    ///
    /// This is the VFS-style approach: each layer is extracted once and cached,
    /// then stacked using copy-based mounts.
    ///
    /// # Returns
    /// Vector of paths to extracted layer directories, ordered bottom to top.
    /// Each path is a directory containing the extracted layer contents.
    ///
    /// # Example
    /// ```ignore
    /// let extracted = image.layer_extracted().await?;
    /// // extracted[0] = /images/extracted/sha256:abc.../  (base layer)
    /// // extracted[1] = /images/extracted/sha256:def.../  (layer 1)
    /// // extracted[2] = /images/extracted/sha256:ghi.../  (layer 2)
    /// ```
    pub async fn layer_extracted(&self) -> BoxliteResult<Vec<PathBuf>> {
        let digests: Vec<String> = self
            .manifest
            .layers
            .iter()
            .map(|l| l.digest.clone())
            .collect();

        self.store.layer_extracted(digests).await
    }

    /// Compute a stable digest for this image based on its layers.
    ///
    /// This is used as a cache key for base disks - same layers = same base disk.
    /// Uses SHA256 hash of concatenated layer digests.
    pub(crate) fn compute_image_digest(&self) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        for layer in &self.manifest.layers {
            hasher.update(layer.digest.as_bytes());
        }
        format!("sha256:{:x}", hasher.finalize())
    }

    /// Get existing disk image if available.
    ///
    /// Returns a persistent Disk if the cached disk image exists, None otherwise.
    /// Does not create a new disk image - use for cache lookups only.
    pub async fn disk_image(&self) -> Option<crate::disk::Disk> {
        let image_digest = self.compute_image_digest();
        self.store.disk_image(&image_digest).await
    }

    /// Install a disk as the cached disk image for this image.
    ///
    /// Atomically moves the source disk to the image store path.
    /// The source disk is consumed and a new persistent Disk is returned.
    pub async fn install_disk_image(
        &self,
        disk: crate::disk::Disk,
    ) -> boxlite_shared::BoxliteResult<crate::disk::Disk> {
        let image_digest = self.compute_image_digest();
        self.store.install_disk_image(&image_digest, disk).await
    }

    // ========================================================================
    // INSPECTION
    // ========================================================================

    /// Pretty-print image information
    #[allow(dead_code)]
    pub fn inspect(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!("{}\n", self.reference));
        output.push_str(&format!("Config: {}\n", self.config_digest()));
        output.push_str(&format!("Layers ({}):\n", self.layer_count()));

        for (i, layer) in self.manifest.layers.iter().enumerate() {
            output.push_str(&format!("  {}. {}\n", i + 1, layer.digest));
        }

        output
    }
}

impl std::fmt::Debug for ImageObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageObject")
            .field("reference", &self.reference)
            .field("layers", &self.manifest.layers.len())
            .field("config_digest", &self.manifest.config_digest)
            .finish()
    }
}

impl std::fmt::Display for ImageObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} layers)",
            self.reference,
            self.manifest.layers.len()
        )
    }
}
