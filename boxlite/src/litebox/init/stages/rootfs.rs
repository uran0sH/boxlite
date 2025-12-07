//! Stage 2: Rootfs preparation.
//!
//! Pulls container image and prepares rootfs (merged or overlayfs layers).

use crate::images::ContainerConfig;
use crate::litebox::init::types::{RootfsInput, RootfsOutput, RootfsPrepResult, USE_OVERLAYFS};
use boxlite_shared::errors::{BoxliteError, BoxliteResult};

/// Pull image and prepare rootfs.
///
/// **Single Responsibility**: Image pulling + rootfs preparation.
pub async fn run(input: RootfsInput<'_>) -> BoxliteResult<RootfsOutput> {
    let image_ref = match &input.options.rootfs {
        crate::runtime::options::RootfsSpec::Image(r) => r,
        crate::runtime::options::RootfsSpec::RootfsPath(_) => {
            return Err(BoxliteError::Storage(
                "Direct rootfs paths not yet supported".into(),
            ));
        }
    };

    // Pull image
    let image = pull_image(input.runtime, image_ref).await?;

    // Prepare rootfs
    let rootfs_result = if USE_OVERLAYFS {
        prepare_overlayfs_layers(&image).await?
    } else {
        // For merged rootfs, we need the layout - but we run in parallel with filesystem stage.
        // The merged rootfs path will be created in config stage where we have layout.
        // For now, overlayfs is the default and recommended approach.
        return Err(BoxliteError::Storage(
            "Merged rootfs not supported in parallel pipeline. Use overlayfs (default).".into(),
        ));
    };

    // Load container config
    let image_config = image.load_config().await?;
    let mut container_config = ContainerConfig::from_oci_config(&image_config)?;

    // Merge user environment variables
    if !input.options.env.is_empty() {
        container_config.merge_env(input.options.env.clone());
    }

    Ok(RootfsOutput {
        container_config,
        rootfs_result,
        image,
    })
}

async fn pull_image(
    runtime: &crate::runtime::RuntimeInner,
    image_ref: &str,
) -> BoxliteResult<crate::images::ImageObject> {
    let image_manager = {
        let state = runtime.acquire_read()?;
        state.image_manager.clone()
    };
    image_manager.pull(image_ref).await
}

async fn prepare_overlayfs_layers(
    image: &crate::images::ImageObject,
) -> BoxliteResult<RootfsPrepResult> {
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

    Ok(RootfsPrepResult::Layers {
        layers_dir,
        layer_names,
    })
}
