//! Task: Container rootfs preparation.
//!
//! Pulls container image and prepares container rootfs:
//! - Disk-based: Creates ext4 disk image from merged layers (fast boot)
//! - Overlayfs: Extracts layers for guest-side overlayfs (flexible)
//!
//! For restart (reuse_rootfs=true), opens existing COW disk instead of creating new.

use super::{InitCtx, log_task_error, task_start};
use crate::disk::{BackingFormat, Disk, DiskFormat, Qcow2Helper, create_ext4_from_dir};
use crate::images::ContainerImageConfig;
use crate::litebox::init::types::{ContainerRootfsPrepResult, USE_DISK_ROOTFS, USE_OVERLAYFS};
use crate::pipeline::PipelineTask;
use crate::runtime::layout::BoxFilesystemLayout;
use crate::runtime::options::RootfsSpec;
use crate::runtime::rt_impl::SharedRuntimeImpl;
use async_trait::async_trait;
use boxlite_shared::errors::{BoxliteError, BoxliteResult};

pub struct ContainerRootfsTask;

#[async_trait]
impl PipelineTask<InitCtx> for ContainerRootfsTask {
    async fn run(self: Box<Self>, ctx: InitCtx) -> BoxliteResult<()> {
        let task_name = self.name();
        let box_id = task_start(&ctx, task_name).await;

        let (rootfs_spec, env, runtime, layout, reuse_rootfs, disk_size_gb) = {
            let ctx = ctx.lock().await;
            let layout = ctx
                .layout
                .clone()
                .ok_or_else(|| BoxliteError::Internal("filesystem task must run first".into()))?;
            (
                ctx.config.options.rootfs.clone(),
                ctx.config.options.env.clone(),
                ctx.runtime.clone(),
                layout,
                ctx.reuse_rootfs,
                ctx.config.options.disk_size_gb,
            )
        };

        let (container_image_config, disk) = run_container_rootfs(
            &rootfs_spec,
            &env,
            &runtime,
            &layout,
            reuse_rootfs,
            disk_size_gb,
        )
        .await
        .inspect_err(|e| log_task_error(&box_id, task_name, e))?;

        let mut ctx = ctx.lock().await;
        ctx.container_image_config = Some(container_image_config);
        ctx.container_disk = Some(disk);

        Ok(())
    }

    fn name(&self) -> &str {
        "container_rootfs_prep"
    }
}

/// Pull image and prepare rootfs, then create or reuse COW disk.
async fn run_container_rootfs(
    rootfs_spec: &RootfsSpec,
    env: &[(String, String)],
    runtime: &SharedRuntimeImpl,
    layout: &BoxFilesystemLayout,
    reuse_rootfs: bool,
    disk_size_gb: Option<u64>,
) -> BoxliteResult<(ContainerImageConfig, Disk)> {
    let disk_path = layout.disk_path();

    // For restart, reuse existing COW disk
    if reuse_rootfs {
        tracing::info!(
            disk_path = %disk_path.display(),
            "Restart mode: reusing existing container rootfs disk"
        );

        if !disk_path.exists() {
            return Err(BoxliteError::Storage(format!(
                "Cannot restart: container rootfs disk not found at {}",
                disk_path.display()
            )));
        }

        let disk = Disk::new(disk_path.clone(), DiskFormat::Qcow2, true);

        // Load container config
        let container_image_config = match rootfs_spec {
            RootfsSpec::Image(r) => {
                let image = pull_image(runtime, r).await?;
                let image_config = image.load_config().await?;
                let mut config = ContainerImageConfig::from_oci_config(&image_config)?;
                if !env.is_empty() {
                    config.merge_env(env.to_vec());
                }
                config
            }
            RootfsSpec::RootfsPath(path) => {
                let bundle_dir = std::path::Path::new(path);

                if !bundle_dir.exists() {
                    return Err(BoxliteError::Storage(format!(
                        "Rootfs path does not exist: {}",
                        path
                    )));
                }

                let (config, _) = load_oci_image_layout(bundle_dir, runtime).await?;
                let mut config = config;
                if !env.is_empty() {
                    config.merge_env(env.to_vec());
                }
                config
            }
        };

        return Ok((container_image_config, disk));
    }

    // Fresh start: pull image and prepare rootfs
    let (container_image_config, rootfs_result) = match rootfs_spec {
        RootfsSpec::Image(r) => {
            let image = pull_image(runtime, r).await?;

            let rootfs_result = if USE_DISK_ROOTFS {
                prepare_disk_rootfs(runtime, &image).await?
            } else if USE_OVERLAYFS {
                prepare_overlayfs_layers(&image).await?
            } else {
                return Err(BoxliteError::Storage(
                    "Merged rootfs not supported. Use overlayfs or disk rootfs.".into(),
                ));
            };

            let image_config = image.load_config().await?;
            let mut config = ContainerImageConfig::from_oci_config(&image_config)?;

            if !env.is_empty() {
                config.merge_env(env.to_vec());
            }

            (config, rootfs_result)
        }
        RootfsSpec::RootfsPath(path) => {
            let bundle_dir = std::path::Path::new(path);

            if !bundle_dir.exists() {
                return Err(BoxliteError::Storage(format!(
                    "Rootfs path does not exist: {}",
                    path
                )));
            }

            // Load from OCI Image Layout format
            let (config, rootfs_result) = load_oci_image_layout(bundle_dir, runtime).await?;

            // Merge user-provided environment
            let mut config = config;
            if !env.is_empty() {
                config.merge_env(env.to_vec());
            }

            (config, rootfs_result)
        }
    };

    let disk = create_cow_disk(&rootfs_result, layout, disk_size_gb)?;

    Ok((container_image_config, disk))
}

/// Create COW disk from base rootfs.
///
/// # Arguments
/// * `rootfs_result` - Result of rootfs preparation (disk image or layers)
/// * `layout` - Box filesystem layout for disk paths
/// * `disk_size_gb` - Optional user-specified disk size in GB. If set, the COW disk
///   will have this virtual size (or the base disk size, whichever is larger).
fn create_cow_disk(
    rootfs_result: &ContainerRootfsPrepResult,
    layout: &crate::runtime::layout::BoxFilesystemLayout,
    disk_size_gb: Option<u64>,
) -> BoxliteResult<Disk> {
    match rootfs_result {
        ContainerRootfsPrepResult::DiskImage {
            base_disk_path,
            disk_size: base_disk_size,
        } => {
            // Calculate target disk size: use max of user-specified size and base disk size
            let target_disk_size = if let Some(size_gb) = disk_size_gb {
                let user_size_bytes = size_gb * 1024 * 1024 * 1024;
                std::cmp::max(user_size_bytes, *base_disk_size)
            } else {
                *base_disk_size
            };

            let qcow2_helper = Qcow2Helper::new();
            let cow_disk_path = layout.disk_path();
            let temp_disk = qcow2_helper.create_cow_child_disk(
                base_disk_path,
                BackingFormat::Raw,
                &cow_disk_path,
                target_disk_size,
            )?;

            // Make disk persistent so it survives stop/restart
            // create_cow_child_disk returns non-persistent disk, but we want to preserve
            // COW disks across box restarts (only delete on remove)
            let disk_path = temp_disk.leak(); // Prevent cleanup
            let disk = Disk::new(disk_path, DiskFormat::Qcow2, true); // persistent=true

            tracing::info!(
                cow_disk = %cow_disk_path.display(),
                base_disk = %base_disk_path.display(),
                virtual_size_mb = target_disk_size / (1024 * 1024),
                "Created container rootfs COW overlay (persistent)"
            );

            Ok(disk)
        }
        ContainerRootfsPrepResult::Layers { .. } => Err(BoxliteError::Internal(
            "Layers mode requires overlayfs - disk creation not applicable".into(),
        )),
        ContainerRootfsPrepResult::Merged(_) => {
            Err(BoxliteError::Internal("Merged mode not supported".into()))
        }
    }
}

async fn pull_image(
    runtime: &crate::runtime::SharedRuntimeImpl,
    image_ref: &str,
) -> BoxliteResult<crate::images::ImageObject> {
    // ImageManager has internal locking - direct access
    runtime.image_manager.pull(image_ref).await
}

async fn prepare_overlayfs_layers(
    image: &crate::images::ImageObject,
) -> BoxliteResult<ContainerRootfsPrepResult> {
    let layer_paths = image.layer_extracted().await?;

    if layer_paths.is_empty() {
        return Err(BoxliteError::Storage(
            "No layers found for overlayfs".into(),
        ));
    }

    let layers_dir = layer_paths[0]
        .parent()
        .ok_or_else(|| BoxliteError::Storage("Layer path has no parent directory".into()))?
        .to_path_buf();

    let layer_names: Vec<String> = layer_paths
        .iter()
        .map(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        })
        .collect();

    tracing::info!(
        "Prepared {} layers for guest-side overlayfs",
        layer_names.len()
    );

    Ok(ContainerRootfsPrepResult::Layers {
        layers_dir,
        layer_names,
    })
}

/// Prepare disk-based rootfs from image layers.
///
/// This function:
/// 1. Checks if a cached base disk image exists for this image
/// 2. If not, merges layers and creates an ext4 disk image
/// 3. Returns the path to the base disk for COW overlay creation
async fn prepare_disk_rootfs(
    runtime: &crate::runtime::SharedRuntimeImpl,
    image: &crate::images::ImageObject,
) -> BoxliteResult<ContainerRootfsPrepResult> {
    // Check if we already have a cached disk image for this image
    if let Some(disk) = image.disk_image().await {
        let disk_path = disk.path().to_path_buf();
        let disk_size = std::fs::metadata(&disk_path)
            .map(|m| m.len())
            .unwrap_or(64 * 1024 * 1024);

        tracing::info!(
            "Using cached disk image: {} ({}MB)",
            disk_path.display(),
            disk_size / (1024 * 1024)
        );

        // Leak the disk to prevent cleanup (it's a cached persistent disk)
        let _ = disk.leak();

        return Ok(ContainerRootfsPrepResult::DiskImage {
            base_disk_path: disk_path,
            disk_size,
        });
    }

    // No cached disk - we need to create one from layers
    tracing::info!("Creating disk image from layers (first run for this image)");

    // Step 1: Extract and merge layers using RootfsBuilder
    let layer_paths = image.layer_extracted().await?;

    if layer_paths.is_empty() {
        return Err(BoxliteError::Storage(
            "No layers found for disk rootfs".into(),
        ));
    }

    // Create a temporary directory for merged rootfs within boxlite home (same filesystem as destination)
    let temp_base = runtime.layout.temp_dir();
    let temp_dir = tempfile::tempdir_in(&temp_base)
        .map_err(|e| BoxliteError::Storage(format!("Failed to create temp directory: {}", e)))?;
    let merged_path = temp_dir.path().join("merged");

    // Use RootfsBuilder to merge layers
    let builder = crate::rootfs::RootfsBuilder::new();
    let _prepared = builder.prepare(merged_path.clone(), image).await?;

    tracing::info!(
        "Merged {} layers into temporary directory",
        layer_paths.len()
    );

    // Step 2: Create ext4 disk image from merged rootfs
    let temp_disk_path = temp_dir.path().join("rootfs.ext4");

    // Use blocking spawn for sync disk creation
    let merged_clone = merged_path.clone();
    let disk_path_clone = temp_disk_path.clone();
    let temp_disk =
        tokio::task::spawn_blocking(move || create_ext4_from_dir(&merged_clone, &disk_path_clone))
            .await
            .map_err(|e| BoxliteError::Internal(format!("Disk creation task failed: {}", e)))??;

    let disk_size = std::fs::metadata(temp_disk.path())
        .map(|m| m.len())
        .unwrap_or(64 * 1024 * 1024);

    tracing::info!(
        "Created ext4 disk image: {} ({}MB)",
        temp_disk.path().display(),
        disk_size / (1024 * 1024)
    );

    // Step 3: Install disk image to cache
    let installed_disk = image.install_disk_image(temp_disk).await?;
    let final_path = installed_disk.path().to_path_buf();

    // Leak the disk to prevent cleanup
    let _ = installed_disk.leak();

    tracing::info!("Installed disk image to cache: {}", final_path.display());

    // Cleanup: temp_dir is dropped automatically

    Ok(ContainerRootfsPrepResult::DiskImage {
        base_disk_path: final_path,
        disk_size,
    })
}

/// Load rootfs from OCI Image Layout format.
///
/// Expected structure:
///   index.json        - OCI index pointing to manifest digest
///   blobs/sha256/     - Content-addressed blobs (manifest, config, layers)
///
/// This is the standard OCI image layout format, typically produced by:
/// - `podman save --format=oci-dir`
/// - `skopeo copy docker://alpine oci:alpine-dir`
/// - Extracted OCI archives
async fn load_oci_image_layout(
    bundle_dir: &std::path::Path,
    runtime: &SharedRuntimeImpl,
) -> BoxliteResult<(ContainerImageConfig, ContainerRootfsPrepResult)> {
    tracing::info!("Loading OCI image layout from: {}", bundle_dir.display());

    // 1. Load index.json (entry point)
    let index_path = bundle_dir.join("index.json");
    if !index_path.exists() {
        return Err(BoxliteError::Storage(format!(
            "OCI image layout must contain index.json, not found at: {}",
            index_path.display()
        )));
    }

    let index_json = std::fs::read_to_string(&index_path)
        .map_err(|e| BoxliteError::Storage(format!("Failed to read index.json: {}", e)))?;

    let index: oci_spec::image::ImageIndex = serde_json::from_str(&index_json)
        .map_err(|e| BoxliteError::Storage(format!("Failed to parse index.json: {}", e)))?;

    // 2. Get manifest from index (use first manifest)
    let manifest_entry = index
        .manifests()
        .first()
        .ok_or_else(|| BoxliteError::Storage("No manifests in index.json".into()))?;

    let manifest_digest = manifest_entry.digest().digest();
    tracing::debug!("Loading manifest: {}", manifest_digest);

    // 3. Load manifest from blobs
    let manifest_path = blob_path(bundle_dir, manifest_digest)?;
    let manifest_json = std::fs::read_to_string(&manifest_path)
        .map_err(|e| BoxliteError::Storage(format!("Failed to read manifest: {}", e)))?;

    let manifest: oci_spec::image::ImageManifest = serde_json::from_str(&manifest_json)
        .map_err(|e| BoxliteError::Storage(format!("Failed to parse manifest: {}", e)))?;

    // 4. Load config from manifest
    let config_digest = manifest.config().digest().digest();
    tracing::debug!("Loading config: {}", config_digest);

    let config_path = blob_path(bundle_dir, config_digest)?;
    let config_json = std::fs::read_to_string(&config_path)
        .map_err(|e| BoxliteError::Storage(format!("Failed to read config blob: {}", e)))?;

    let oci_config: oci_spec::image::ImageConfiguration = serde_json::from_str(&config_json)
        .map_err(|e| BoxliteError::Storage(format!("Failed to parse OCI config: {}", e)))?;

    let container_config = ContainerImageConfig::from_oci_config(&oci_config)?;

    // 5. Get layer tarballs from manifest
    let layer_paths: Vec<std::path::PathBuf> = manifest
        .layers()
        .iter()
        .map(|layer_desc| {
            let digest = layer_desc.digest().digest();
            blob_path(bundle_dir, digest)
        })
        .collect::<BoxliteResult<Vec<_>>>()?;

    if layer_paths.is_empty() {
        return Err(BoxliteError::Storage("No layers in manifest".into()));
    }

    tracing::info!("Found {} layers in OCI image layout", layer_paths.len());

    // 6. Extract and merge layers, create disk
    let rootfs_result = prepare_disk_from_oci_layers(&layer_paths, runtime).await?;

    Ok((container_config, rootfs_result))
}

/// Get blob path from digest or hash.
///
/// Converts:
/// - "sha256:abc123..." -> "bundle_dir/blobs/sha256/abc123..."
/// - "abc123..." (hash only) -> "bundle_dir/blobs/sha256/abc123..."
fn blob_path(
    bundle_dir: &std::path::Path,
    digest_or_hash: &str,
) -> BoxliteResult<std::path::PathBuf> {
    // If already has prefix, use it; otherwise assume sha256
    let (algorithm, hash) = if digest_or_hash.contains(':') {
        let parts: Vec<&str> = digest_or_hash.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(BoxliteError::Storage(format!(
                "Invalid digest format: {}",
                digest_or_hash
            )));
        }
        (parts[0], parts[1])
    } else {
        // Assume sha256 if no prefix
        ("sha256", digest_or_hash)
    };

    if algorithm != "sha256" {
        return Err(BoxliteError::Storage(format!(
            "Unsupported digest algorithm: {} (only sha256 supported)",
            algorithm
        )));
    }

    Ok(bundle_dir.join("blobs").join(algorithm).join(hash))
}

/// Prepare disk image from OCI layer tarballs.
///
/// Extracts and merges layers in order, then creates ext4 disk image.
async fn prepare_disk_from_oci_layers(
    layer_tarballs: &[std::path::PathBuf],
    runtime: &SharedRuntimeImpl,
) -> BoxliteResult<ContainerRootfsPrepResult> {
    // Create temporary directory for merged rootfs
    let temp_base = runtime.layout.temp_dir();
    let temp_dir = tempfile::tempdir_in(&temp_base)
        .map_err(|e| BoxliteError::Storage(format!("Failed to create temp directory: {}", e)))?;
    let merged_path = temp_dir.path().join("merged");

    std::fs::create_dir_all(&merged_path)
        .map_err(|e| BoxliteError::Storage(format!("Failed to create merged directory: {}", e)))?;

    // Extract layers in order (each layer overwrites previous)
    for (i, tarball) in layer_tarballs.iter().enumerate() {
        tracing::debug!(
            "Extracting layer {}/{}: {}",
            i + 1,
            layer_tarballs.len(),
            tarball.display()
        );

        crate::images::extract_layer_tarball_streaming(tarball, &merged_path)?;
    }

    tracing::info!(
        "Merged {} layers into: {}",
        layer_tarballs.len(),
        merged_path.display()
    );

    // Create ext4 disk image
    let temp_disk_path = temp_dir.path().join("rootfs.ext4");
    let merged_clone = merged_path.clone();
    let disk_path_clone = temp_disk_path.clone();

    let temp_disk =
        tokio::task::spawn_blocking(move || create_ext4_from_dir(&merged_clone, &disk_path_clone))
            .await
            .map_err(|e| BoxliteError::Internal(format!("Disk creation task failed: {}", e)))??;

    let disk_size = std::fs::metadata(&temp_disk_path)
        .map(|m| m.len())
        .unwrap_or(64 * 1024 * 1024);

    tracing::info!(
        "Created ext4 disk image from OCI layers: {} ({}MB)",
        temp_disk_path.display(),
        disk_size / (1024 * 1024)
    );

    // Leak the disk to prevent cleanup (the file will be used as backing for COW disk)
    let _ = temp_disk.leak();

    // Prevent temp_dir from being cleaned up when the function returns
    // keep() consumes the TempDir and returns the PathBuf, preventing automatic deletion
    let _kept_path = temp_dir.keep();

    Ok(ContainerRootfsPrepResult::DiskImage {
        base_disk_path: temp_disk_path,
        disk_size,
    })
}
