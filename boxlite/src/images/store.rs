//! Thread-safe OCI image store.
//!
//! This module provides `ImageStore`, a thread-safe facade over image storage
//! that handles locking internally. Users don't need to manage locks.
//!
//! Architecture:
//! - `ImageStoreInner`: Mutable state (index, storage) - no locking awareness
//! - `ImageStore`: Thread-safe wrapper with `RwLock<ImageStoreInner>`
//!
//! Public API (Option C - minimal, noun-ish):
//! - `pull()` - Pull image from registry (or return cached)
//! - `config()` - Load config JSON
//! - `layer_tarball()` - Get layer tarball path
//! - `layer_extracted()` - Get extracted layer path (extracts if needed)

use crate::db::{CachedImage, Database, ImageIndexStore};
use crate::images::manager::{ImageManifest, LayerInfo};
use crate::images::storage::ImageStorage;
use boxlite_shared::{BoxliteError, BoxliteResult};
use oci_client::Reference;
use oci_client::manifest::OciDescriptor;
use oci_client::secrets::RegistryAuth;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// ============================================================================
// INNER STATE (no locking awareness)
// ============================================================================

/// Mutable state for image operations.
///
/// This struct contains all mutable state but has NO locking - it's wrapped
/// by `ImageStore` which provides thread-safe access.
struct ImageStoreInner {
    index: ImageIndexStore,
    storage: ImageStorage,
}

impl ImageStoreInner {
    fn new(images_dir: PathBuf, db: Database) -> BoxliteResult<Self> {
        let storage = ImageStorage::new(images_dir)?;
        let index = ImageIndexStore::new(db);
        Ok(Self { index, storage })
    }
}

// ============================================================================
// IMAGE STORE (thread-safe facade)
// ============================================================================

/// Thread-safe OCI image store.
///
/// Provides a simple, thread-safe API for image operations. Locking is handled
/// internally - callers don't need to manage locks.
///
/// # Thread Safety
///
/// - `pull()`: Releases lock during network I/O for better concurrency
/// - `config()`, `layer_tarball()`: Quick read operations
/// - `layer_extracted()`: May do I/O but uses atomic file operations
///
/// # Example
///
/// ```ignore
/// let store = Arc::new(ImageStore::new(images_dir)?);
///
/// // Pull image (thread-safe, releases lock during download)
/// let manifest = store.pull("python:alpine").await?;
///
/// // Access layer data
/// let tarball = store.layer_tarball(&manifest.layers[0].digest);
/// let extracted = store.layer_extracted(&manifest.layers[0].digest)?;
/// ```
pub struct ImageStore {
    /// OCI registry client (immutable, outside lock)
    client: oci_client::Client,
    /// Mutable state protected by RwLock
    inner: RwLock<ImageStoreInner>,
    /// Registries to search for unqualified image references.
    /// Tried in order; first successful pull wins.
    registries: Vec<String>,
}

impl std::fmt::Debug for ImageStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageStore").finish()
    }
}

impl ImageStore {
    /// Create a new image store for the given images' directory.
    ///
    /// # Arguments
    /// * `images_dir` - Directory for image cache
    /// * `db` - Database for image index
    /// * `registries` - Registries to search for unqualified images (tried in order)
    pub fn new(images_dir: PathBuf, db: Database, registries: Vec<String>) -> BoxliteResult<Self> {
        let inner = ImageStoreInner::new(images_dir, db)?;
        Ok(Self {
            client: oci_client::Client::new(Default::default()),
            inner: RwLock::new(inner),
            registries,
        })
    }

    // ========================================================================
    // PUBLIC API
    // ========================================================================

    /// Pull an image from registry (or return cached manifest).
    ///
    /// This method:
    /// 1. Parses and resolves image reference using configured registries
    /// 2. Checks local cache for each candidate (quick read lock)
    /// 3. If not cached, downloads from registry (releases lock during I/O)
    /// 4. Tries each registry candidate in order until one succeeds
    ///
    /// Thread-safe: Multiple concurrent pulls of the same image will only
    /// download once; others will get the cached result.
    pub async fn pull(&self, image_ref: &str) -> BoxliteResult<ImageManifest> {
        use super::ReferenceIter;

        tracing::debug!(
            image_ref = %image_ref,
            registries = ?self.registries,
            "Starting image pull with registry fallback"
        );

        // Parse image reference and create iterator over registry candidates
        let candidates = ReferenceIter::new(image_ref, &self.registries)
            .map_err(|e| BoxliteError::Storage(format!("invalid image reference: {e}")))?;

        let mut errors: Vec<(String, BoxliteError)> = Vec::new();

        for reference in candidates {
            let ref_str = reference.whole();

            // Fast path: check cache with read lock
            {
                let inner = self.inner.read().await;
                if let Some(manifest) = self.try_load_cached(&inner, &ref_str)? {
                    tracing::info!("Using cached image: {}", ref_str);
                    return Ok(manifest);
                }
            } // Read lock released

            // Slow path: pull from registry
            tracing::info!("Pulling image from registry: {}", ref_str);
            match self.pull_from_registry(&reference).await {
                Ok(manifest) => {
                    if !errors.is_empty() {
                        tracing::info!(
                            original = %image_ref,
                            resolved = %ref_str,
                            "Successfully pulled image after {} attempts",
                            errors.len() + 1
                        );
                    }
                    return Ok(manifest);
                }
                Err(e) => {
                    tracing::debug!(
                        reference = %ref_str,
                        error = %e,
                        "Failed to pull image candidate, trying next"
                    );
                    errors.push((ref_str, e));
                }
            }
        }

        // All candidates failed - format comprehensive error message
        if errors.is_empty() {
            Err(BoxliteError::Storage(format!(
                "No registries configured for image: {}",
                image_ref
            )))
        } else {
            let details: Vec<String> = errors
                .iter()
                .map(|(registry, err)| format!("  - {}: {}", registry, err))
                .collect();

            Err(BoxliteError::Storage(format!(
                "Failed to pull image '{}' after trying {} {}:\n{}",
                image_ref,
                errors.len(),
                if errors.len() == 1 {
                    "registry"
                } else {
                    "registries"
                },
                details.join("\n")
            )))
        }
    }

    /// Load an OCI/Docker image from a local directory.
    ///
    /// Reads image manifest from manifest.json and returns an `ImageManifest`.
    /// Layers and configs are imported into the image store using hard links.
    ///
    /// Expected structure:
    ///   ```text
    ///   {path}/
    ///     manifest.json     - Docker/OCI manifest with Config and Layers paths
    ///     blobs/sha256/     - Content-addressed blobs
    ///       {config_digest}
    ///       {layer_digest_1}
    ///       {layer_digest_2}
    ///       ...
    ///   ```
    ///
    /// The manifest.json format:
    ///   ```json
    ///   [{
    ///     "Config": "blobs/sha256/abc123...",
    ///     "Layers": ["blobs/sha256/def456...", ...]
    ///   }]
    ///   ```
    ///
    /// # Arguments
    /// * `path` - Path to local image directory
    ///
    /// # Returns
    /// `ImageManifest` with layer digests and config digest
    ///
    /// # Errors
    /// - If `path/manifest.json` doesn't exist
    /// - If any referenced blob is missing
    /// - If hard linking fails
    pub async fn load_from_local(&self, path: std::path::PathBuf) -> BoxliteResult<ImageManifest> {
        tracing::info!("Loading image from local path: {}", path.display());

        // 1. Load manifest.json
        let manifest_path = path.join("manifest.json");
        if !manifest_path.exists() {
            return Err(BoxliteError::Storage(format!(
                "Local image must contain manifest.json, not found at: {}",
                manifest_path.display()
            )));
        }

        let manifest_json = std::fs::read_to_string(&manifest_path)
            .map_err(|e| BoxliteError::Storage(format!("Failed to read manifest.json: {}", e)))?;

        // Parse manifest format: array of manifest objects
        let manifests: Vec<serde_json::Value> = serde_json::from_str(&manifest_json)
            .map_err(|e| BoxliteError::Storage(format!("Failed to parse manifest.json: {}", e)))?;

        let manifest_obj = manifests
            .first()
            .ok_or_else(|| BoxliteError::Storage("Empty manifest.json".into()))?;

        // 2. Extract config path and convert to digest
        let config_path = manifest_obj
            .get("Config")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BoxliteError::Storage("Manifest missing Config field".into()))?;

        // "blobs/sha256/abc..." -> "sha256:abc..."
        let config_digest = config_path.replace('/', ":");

        // 3. Extract layer paths and convert to digests
        let layers_arr = manifest_obj
            .get("Layers")
            .and_then(|v| v.as_array())
            .ok_or_else(|| BoxliteError::Storage("Manifest missing Layers field".into()))?;

        let mut layers = Vec::new();
        for layer_path in layers_arr {
            let path_str = layer_path
                .as_str()
                .ok_or_else(|| BoxliteError::Storage("Layer path is not a string".into()))?;
            let digest = path_str.replace('/', ":");
            layers.push(LayerInfo {
                digest,
                media_type: "application/vnd.oci.image.layer.v1.tar".to_string(),
            });
        }

        // 4. Import config and layers into storage (hard link)
        {
            let inner = self.inner.write().await;

            // Import config blob (configs stored in configs_dir, not layers_dir)
            let config_blob_path = path.join(config_path);
            if !config_blob_path.exists() {
                return Err(BoxliteError::Storage(format!(
                    "Config blob not found: {}",
                    config_blob_path.display()
                )));
            }
            import_config_to_storage(&inner.storage, &config_blob_path, &config_digest)?;

            // Import layer blobs
            for layer in &layers {
                let layer_path = layer.digest.replace(':', "/");
                let layer_blob_path = path.join(&layer_path);

                if !layer_blob_path.exists() {
                    return Err(BoxliteError::Storage(format!(
                        "Layer blob not found: {}",
                        layer_blob_path.display()
                    )));
                }

                import_blob_to_storage(&inner.storage, &layer_blob_path, &layer.digest)?;
            }
        }

        // Generate manifest digest from content
        use sha2::Digest;
        let manifest_digest = format!(
            "sha256:{:x}",
            sha2::Sha256::digest(manifest_json.as_bytes())
        );

        tracing::info!(
            "Loaded local image: config={}, {} layers",
            config_digest,
            layers.len()
        );

        Ok(ImageManifest {
            manifest_digest,
            layers,
            config_digest,
        })
    }

    /// Load config JSON for an image.
    ///
    /// Returns the raw JSON string. Use `serde_json::from_str()` to parse.
    pub async fn config(&self, config_digest: &str) -> BoxliteResult<String> {
        let inner = self.inner.read().await;
        inner.storage.load_config(config_digest)
    }

    /// Get path to layer tarball.
    ///
    /// Returns the path where the layer tarball is stored. The layer must
    /// have been downloaded via `pull()` first.
    pub async fn layer_tarball(&self, digest: &str) -> PathBuf {
        let inner = self.inner.read().await;
        inner.storage.layer_tarball_path(digest)
    }

    /// Get paths to extracted layer directories.
    ///
    /// Extracts layers if not already cached. Uses rayon for parallel extraction
    /// and atomic file operations so concurrent calls are safe.
    ///
    /// # Arguments
    /// * `digests` - Layer digests to extract (ordered bottom to top)
    ///
    /// # Returns
    /// Vector of paths to extracted layer directories (same order as input)
    pub async fn layer_extracted(&self, digests: Vec<String>) -> BoxliteResult<Vec<PathBuf>> {
        use rayon::prelude::*;

        // Get all paths with read lock
        let layer_info: Vec<(String, PathBuf, PathBuf)> = {
            let inner = self.inner.read().await;
            digests
                .iter()
                .map(|digest| {
                    (
                        digest.clone(),
                        inner.storage.layer_tarball_path(digest),
                        inner.storage.layer_extracted_path(digest),
                    )
                })
                .collect()
        }; // Lock released

        // Extract layers in parallel using rayon (sync operations)
        // extract_layer uses atomic file operations so concurrent calls are safe
        let inner = self.inner.read().await;
        layer_info
            .into_par_iter()
            .map(|(digest, tarball_path, extracted_path)| {
                // Check if already extracted
                if extracted_path.exists() {
                    tracing::debug!("Using cached extracted layer: {}", digest);
                    return Ok(extracted_path);
                }

                // Extract layer (atomic - safe for concurrent access)
                tracing::debug!("Extracting layer: {}", digest);
                inner
                    .storage
                    .extract_layer(digest.as_str(), &tarball_path)?;
                Ok(extracted_path)
            })
            .collect()
    }

    /// Get existing disk image for an image digest if available.
    ///
    /// Returns a persistent Disk if the cached disk image exists, None otherwise.
    /// The returned Disk is persistent (won't be deleted on drop).
    pub async fn disk_image(&self, image_digest: &str) -> Option<crate::disk::Disk> {
        let inner = self.inner.read().await;
        if let Some((path, format)) = inner.storage.find_disk_image(image_digest) {
            Some(crate::disk::Disk::new(path, format, true))
        } else {
            None
        }
    }

    /// Install a disk as the cached disk image for an image digest.
    ///
    /// Atomically moves the source disk to the image store path.
    /// The source disk is consumed and a new persistent Disk is returned.
    /// The target path extension is determined by the disk's format.
    ///
    /// # Arguments
    /// * `image_digest` - Stable digest identifying the image
    /// * `disk` - Source disk to install (will be moved, not copied)
    ///
    /// # Returns
    /// New persistent Disk at the installed location
    pub async fn install_disk_image(
        &self,
        image_digest: &str,
        disk: crate::disk::Disk,
    ) -> BoxliteResult<crate::disk::Disk> {
        let inner = self.inner.read().await;
        let disk_format = disk.format();
        let target_path = inner.storage.disk_image_path(image_digest, disk_format);

        // Ensure parent directory exists
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                BoxliteError::Storage(format!(
                    "Failed to create disk image directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        // If target already exists, just return it (idempotent)
        if target_path.exists() {
            tracing::debug!("Disk image already installed: {}", target_path.display());
            // Leak the source disk to prevent cleanup (it may have been the same file)
            let _ = disk.leak();
            return Ok(crate::disk::Disk::new(target_path, disk_format, true));
        }

        let source_path = disk.path().to_path_buf();

        // Atomic rename (move) - works if on same filesystem
        std::fs::rename(&source_path, &target_path).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to install disk image from {} to {}: {}",
                source_path.display(),
                target_path.display(),
                e
            ))
        })?;

        // Leak the source disk to prevent Drop from trying to delete the old path
        let _ = disk.leak();

        tracing::info!(
            "Installed disk image: {} -> {}",
            source_path.display(),
            target_path.display()
        );

        Ok(crate::disk::Disk::new(target_path, disk_format, true))
    }

    // ========================================================================
    // INTERNAL: Cache Operations
    // ========================================================================

    /// Try to load image from local cache.
    fn try_load_cached(
        &self,
        inner: &ImageStoreInner,
        image_ref: &str,
    ) -> BoxliteResult<Option<ImageManifest>> {
        // Check if image exists in index
        let cached = match inner.index.get(image_ref)? {
            Some(c) if c.complete => c,
            _ => {
                tracing::debug!("Image not in cache or incomplete: {}", image_ref);
                return Ok(None);
            }
        };

        // Verify all files still exist
        if !self.verify_cached_image(inner, &cached)? {
            tracing::warn!(
                "Cached image files missing, will re-download: {}",
                image_ref
            );
            return Ok(None);
        }

        // Load manifest from disk
        let manifest = self.load_manifest_from_disk(inner, &cached)?;
        Ok(Some(manifest))
    }

    fn verify_cached_image(
        &self,
        inner: &ImageStoreInner,
        cached: &CachedImage,
    ) -> BoxliteResult<bool> {
        if !inner.storage.has_manifest(&cached.manifest_digest) {
            tracing::debug!("Manifest file missing: {}", cached.manifest_digest);
            return Ok(false);
        }

        if !inner.storage.verify_blobs_exist(&cached.layers) {
            tracing::debug!("Some layer files missing");
            return Ok(false);
        }

        if !inner.storage.has_config(&cached.config_digest) {
            tracing::debug!("Config blob missing: {}", cached.config_digest);
            return Ok(false);
        }

        Ok(true)
    }

    fn load_manifest_from_disk(
        &self,
        inner: &ImageStoreInner,
        cached: &CachedImage,
    ) -> BoxliteResult<ImageManifest> {
        let manifest = inner.storage.load_manifest(&cached.manifest_digest)?;

        let (layers, config_digest) = match manifest {
            oci_client::manifest::OciManifest::Image(ref img) => {
                let layers = Self::layers_from_image(img);
                let config_digest = img.config.digest.clone();
                (layers, config_digest)
            }
            _ => {
                return Err(BoxliteError::Storage(
                    "cached manifest is not a simple image".into(),
                ));
            }
        };

        Ok(ImageManifest {
            manifest_digest: cached.manifest_digest.clone(),
            layers,
            config_digest,
        })
    }

    // ========================================================================
    // INTERNAL: Registry Operations (releases lock during I/O)
    // ========================================================================

    /// Pull image from registry using a typed Reference.
    ///
    /// This method handles the actual network I/O - manifest pull, layer download, etc.
    /// Lock is released during network I/O to allow other operations.
    async fn pull_from_registry(&self, reference: &Reference) -> BoxliteResult<ImageManifest> {
        // Step 1: Pull manifest (no lock needed - uses self.client)
        let (manifest, manifest_digest_str) = self
            .client
            .pull_manifest(reference, &RegistryAuth::Anonymous)
            .await
            .map_err(|e| BoxliteError::Storage(format!("failed to pull manifest: {e}")))?;

        // Step 2: Save manifest (quick write lock)
        {
            let inner = self.inner.read().await;
            inner
                .storage
                .save_manifest(&manifest, &manifest_digest_str)?;
        }

        // Step 3: Extract image manifest (may pull platform-specific manifest for multi-platform images)
        let image_manifest = self
            .extract_image_manifest(reference, &manifest, manifest_digest_str)
            .await?;

        // Step 4: Download layers (no lock during download, atomic file writes)
        self.download_layers(reference, &image_manifest.layers)
            .await?;

        // Step 5: Download config (no lock during download)
        self.download_config(reference, &image_manifest.config_digest)
            .await?;

        // Step 6: Update index using reference.whole() as the cache key
        self.update_index(&reference.whole(), &image_manifest)
            .await?;

        Ok(image_manifest)
    }

    /// Update index with newly pulled image.
    async fn update_index(&self, image_ref: &str, manifest: &ImageManifest) -> BoxliteResult<()> {
        let inner = self.inner.read().await;

        let cached_image = CachedImage {
            manifest_digest: manifest.manifest_digest.clone(),
            config_digest: manifest.config_digest.clone(),
            layers: manifest.layers.iter().map(|l| l.digest.clone()).collect(),
            cached_at: chrono::Utc::now().to_rfc3339(),
            complete: true,
        };

        inner.index.upsert(image_ref, &cached_image)?;

        tracing::debug!("Updated index for image: {}", image_ref);
        Ok(())
    }

    // ========================================================================
    // INTERNAL: Manifest Parsing
    // ========================================================================

    async fn extract_image_manifest(
        &self,
        reference: &Reference,
        manifest: &oci_client::manifest::OciManifest,
        manifest_digest: String,
    ) -> BoxliteResult<ImageManifest> {
        match manifest {
            oci_client::manifest::OciManifest::Image(img) => {
                let layers = Self::layers_from_image(img);
                let config_digest = img.config.digest.clone();
                Ok(ImageManifest {
                    manifest_digest,
                    layers,
                    config_digest,
                })
            }
            oci_client::manifest::OciManifest::ImageIndex(index) => {
                self.extract_platform_manifest(reference, index).await
            }
        }
    }

    fn layers_from_image(image: &oci_client::manifest::OciImageManifest) -> Vec<LayerInfo> {
        image
            .layers
            .iter()
            .map(|layer| LayerInfo {
                digest: layer.digest.clone(),
                media_type: layer.media_type.clone(),
            })
            .collect()
    }

    async fn extract_platform_manifest(
        &self,
        reference: &Reference,
        index: &oci_client::manifest::OciImageIndex,
    ) -> BoxliteResult<ImageManifest> {
        let (platform_os, platform_arch) = Self::detect_platform();

        tracing::debug!(
            "Image index detected, selecting platform: {}/{} (Rust arch: {})",
            platform_os,
            platform_arch,
            std::env::consts::ARCH
        );

        let platform_manifest = self.select_platform_manifest(index, platform_os, platform_arch)?;

        let platform_ref = format!("{}@{}", reference.whole(), platform_manifest.digest);
        let platform_reference: Reference = platform_ref
            .parse()
            .map_err(|e| BoxliteError::Storage(format!("invalid platform reference: {e}")))?;

        tracing::info!(
            "Pulling platform-specific manifest: {}",
            platform_manifest.digest
        );
        let (platform_image, platform_digest) = self
            .client
            .pull_manifest(&platform_reference, &RegistryAuth::Anonymous)
            .await
            .map_err(|e| BoxliteError::Storage(format!("failed to pull platform manifest: {e}")))?;

        // Save platform manifest (quick lock)
        {
            let inner = self.inner.read().await;
            inner
                .storage
                .save_manifest(&platform_image, &platform_digest)?;
        }

        match platform_image {
            oci_client::manifest::OciManifest::Image(img) => {
                let layers = Self::layers_from_image(&img);
                let config_digest = img.config.digest.clone();
                Ok(ImageManifest {
                    manifest_digest: platform_digest,
                    layers,
                    config_digest,
                })
            }
            _ => Err(BoxliteError::Storage(
                "platform manifest is not a valid image".into(),
            )),
        }
    }

    fn detect_platform() -> (&'static str, &'static str) {
        let os = "linux";
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "amd64",
            "x86" => "386",
            "arm" => "arm",
            other => other,
        };
        (os, arch)
    }

    fn select_platform_manifest<'b>(
        &self,
        index: &'b oci_client::manifest::OciImageIndex,
        platform_os: &str,
        platform_arch: &str,
    ) -> BoxliteResult<&'b oci_client::manifest::ImageIndexEntry> {
        index
            .manifests
            .iter()
            .find(|m| {
                if let Some(p) = &m.platform {
                    p.os == platform_os && p.architecture == platform_arch
                } else {
                    false
                }
            })
            .ok_or_else(|| {
                let available = index
                    .manifests
                    .iter()
                    .filter_map(|m| {
                        m.platform
                            .as_ref()
                            .map(|p| format!("{}/{}", p.os, p.architecture))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                BoxliteError::Storage(format!(
                    "no image found for platform {}/{}. Available platforms: {}",
                    platform_os, platform_arch, available
                ))
            })
    }

    // ========================================================================
    // INTERNAL: Layer Download (no lock during I/O)
    // ========================================================================

    async fn download_layers(
        &self,
        reference: &Reference,
        layers: &[LayerInfo],
    ) -> BoxliteResult<()> {
        use futures::future::join_all;

        // Check which layers need downloading (quick read lock)
        let layers_to_download: Vec<_> = {
            let inner = self.inner.read().await;
            let mut to_download = Vec::new();
            for layer in layers {
                if !inner.storage.has_layer(&layer.digest) {
                    to_download.push(layer.clone());
                } else {
                    // Verify cached layer
                    match inner.storage.verify_layer(&layer.digest).await {
                        Ok(true) => {
                            tracing::debug!("Layer tarball cached and verified: {}", layer.digest);
                        }
                        _ => {
                            tracing::warn!(
                                "Cached layer corrupted, will re-download: {}",
                                layer.digest
                            );
                            let _ = std::fs::remove_file(
                                inner.storage.layer_tarball_path(&layer.digest),
                            );
                            to_download.push(layer.clone());
                        }
                    }
                }
            }
            to_download
        }; // Read lock released

        if layers_to_download.is_empty() {
            return Ok(());
        }

        tracing::info!(
            "Downloading {} layers in parallel",
            layers_to_download.len()
        );

        // Download in parallel (no lock held)
        let download_futures = layers_to_download
            .iter()
            .map(|layer| self.download_layer(reference, layer));

        let results = join_all(download_futures).await;

        for result in results {
            result?;
        }

        Ok(())
    }

    async fn download_layer(&self, reference: &Reference, layer: &LayerInfo) -> BoxliteResult<()> {
        const MAX_RETRIES: u32 = 3;

        tracing::info!("Downloading layer: {}", layer.digest);

        let mut last_error = None;

        for attempt in 1..=MAX_RETRIES {
            if attempt > 1 {
                tracing::info!(
                    "Retrying layer download (attempt {}/{}): {}",
                    attempt,
                    MAX_RETRIES,
                    layer.digest
                );
            }

            // Stage download (quick read lock for path computation)
            let mut staged = {
                let inner = self.inner.read().await;
                match inner.storage.stage_layer_download(&layer.digest).await {
                    Ok(result) => result,
                    Err(e) => {
                        last_error = Some(format!(
                            "Failed to stage layer {} download: {e}",
                            layer.digest
                        ));
                        continue;
                    }
                }
            };

            // Download (no lock)
            match self
                .client
                .pull_blob(
                    reference,
                    &OciDescriptor {
                        digest: layer.digest.clone(),
                        media_type: layer.media_type.clone(),
                        size: 0,
                        urls: None,
                        annotations: None,
                    },
                    staged.file(),
                )
                .await
            {
                Ok(_) => match staged.commit().await {
                    Ok(true) => {
                        tracing::info!("Downloaded and verified layer: {}", layer.digest);
                        return Ok(());
                    }
                    Ok(false) => {
                        tracing::warn!(
                            "Layer integrity check failed (attempt {}): hash mismatch for {}",
                            attempt,
                            layer.digest
                        );
                        last_error =
                            Some("layer integrity verification failed: hash mismatch".to_string());
                    }
                    Err(e) => {
                        tracing::warn!("Layer commit error (attempt {}): {}", attempt, e);
                        last_error = Some(format!("layer commit error: {e}"));
                    }
                },
                Err(e) => {
                    tracing::warn!("Layer download failed (attempt {}): {}", attempt, e);
                    last_error = Some(format!("failed to pull layer {}: {e}", layer.digest));
                    staged.abort().await;
                }
            }
        }

        Err(BoxliteError::Storage(last_error.unwrap_or_else(|| {
            "download failed after retries".to_string()
        })))
    }

    async fn download_config(
        &self,
        reference: &Reference,
        config_digest: &str,
    ) -> BoxliteResult<()> {
        // Check if already cached (quick read lock)
        {
            let inner = self.inner.read().await;
            if inner.storage.has_config(config_digest) {
                tracing::debug!("Config blob already cached: {}", config_digest);
                return Ok(());
            }
        }

        tracing::debug!("Downloading config blob: {}", config_digest);

        // Start staged download (quick read lock)
        let mut staged = {
            let inner = self.inner.read().await;
            inner.storage.stage_config_download(config_digest).await?
        };

        // Download to temp file (no lock)
        if let Err(e) = self
            .client
            .pull_blob(
                reference,
                &OciDescriptor {
                    digest: config_digest.to_string(),
                    media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                    size: 0,
                    urls: None,
                    annotations: None,
                },
                staged.file(),
            )
            .await
        {
            staged.abort().await;
            return Err(BoxliteError::Storage(format!("failed to pull config: {e}")));
        }

        // Verify and commit (atomic move to final location)
        if !staged.commit().await? {
            return Err(BoxliteError::Storage(format!(
                "Config blob verification failed for {}",
                config_digest
            )));
        }

        Ok(())
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Import a single blob into storage from a local path.
///
/// Uses hard links to avoid copying. Falls back to copy if hard link fails.
fn import_blob_to_storage(
    storage: &crate::images::storage::ImageStorage,
    src_path: &std::path::Path,
    digest: &str,
) -> BoxliteResult<()> {
    let dest_path = storage.layer_tarball_path(digest);

    if dest_path.exists() {
        tracing::debug!("Blob already exists: {}", digest);
        return Ok(());
    }

    // Try hard link first (fast, no extra space)
    if let Err(_) = std::fs::hard_link(src_path, &dest_path) {
        // Fallback to copy if hard link fails (different filesystems, etc.)
        tracing::debug!(
            "Hard link failed for {}, copying to {}",
            src_path.display(),
            dest_path.display()
        );
        std::fs::copy(src_path, &dest_path).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to copy blob from {} to {}: {}",
                src_path.display(),
                dest_path.display(),
                e
            ))
        })?;
    }

    tracing::debug!("Imported blob: {} -> {}", digest, dest_path.display());
    Ok(())
}

/// Import a config blob into storage from a local path.
///
/// Uses hard links to avoid copying. Falls back to copy if hard link fails.
fn import_config_to_storage(
    storage: &crate::images::storage::ImageStorage,
    src_path: &std::path::Path,
    digest: &str,
) -> BoxliteResult<()> {
    let dest_path = storage.config_path(digest);

    if dest_path.exists() {
        tracing::debug!("Config blob already exists: {}", digest);
        return Ok(());
    }

    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to create config directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    // Try hard link first (fast, no extra space)
    if let Err(_) = std::fs::hard_link(src_path, &dest_path) {
        // Fallback to copy if hard link fails (different filesystems, etc.)
        tracing::debug!(
            "Hard link failed for config {}, copying to {}",
            src_path.display(),
            dest_path.display()
        );
        std::fs::copy(src_path, &dest_path).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to copy config from {} to {}: {}",
                src_path.display(),
                dest_path.display(),
                e
            ))
        })?;
    }

    tracing::debug!("Imported config: {} -> {}", digest, dest_path.display());
    Ok(())
}

// ============================================================================
// SHARED TYPE ALIAS
// ============================================================================

/// Shared reference to ImageStore.
///
/// Used by `ImageManager` and `ImageObject` to share the same store.
pub type SharedImageStore = Arc<ImageStore>;
