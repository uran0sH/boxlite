//! Stage 3: Init image preparation.
//!
//! Lazily initializes the bootstrap init rootfs (shared across all boxes).

use crate::litebox::init::types::{InitImageInput, InitImageOutput};
use crate::rootfs::{PreparedRootfs, RootfsBuilder};
use crate::runtime::constants::images;
use crate::runtime::initrf::{InitRootfs, Strategy};
use boxlite_shared::errors::{BoxliteError, BoxliteResult};

/// Get or initialize bootstrap init image.
///
/// **Single Responsibility**: Init rootfs lazy initialization.
pub async fn run(input: InitImageInput<'_>) -> BoxliteResult<InitImageOutput> {
    let init_rootfs = input
        .init_rootfs_cell
        .get_or_try_init(|| async {
            tracing::info!(
                "Initializing bootstrap init image {} (first time only)...",
                images::INIT_ROOTFS
            );

            let base_image = pull_init_image(input.runtime).await?;
            let rootfs_dir = input.runtime.non_sync_state.layout.rootfs_dir();

            let prepared = prepare_init_rootfs(&rootfs_dir, &base_image).await?;
            let env = extract_env_from_image(&base_image).await?;

            let init_rootfs = InitRootfs::new(
                prepared.path,
                Strategy::Extracted {
                    layers: base_image.layer_tarballs().await.len(),
                },
                None,
                None,
                env,
            )?;

            tracing::info!(
                "Bootstrap init image ready at {}",
                init_rootfs.path.display()
            );

            Ok::<_, BoxliteError>(init_rootfs)
        })
        .await?;

    Ok(InitImageOutput {
        init_rootfs: init_rootfs.clone(),
    })
}

async fn pull_init_image(
    runtime: &crate::runtime::RuntimeInner,
) -> BoxliteResult<crate::images::ImageObject> {
    let image_manager = {
        let state = runtime.acquire_read()?;
        state.image_manager.clone()
    };
    image_manager.pull(images::INIT_ROOTFS).await
}

async fn prepare_init_rootfs(
    rootfs_dir: &std::path::Path,
    base_image: &crate::images::ImageObject,
) -> BoxliteResult<PreparedRootfs> {
    if rootfs_dir.exists() && rootfs_dir.join("bin").exists() {
        tracing::debug!("Rootfs already exists at {}", rootfs_dir.display());
        Ok(PreparedRootfs {
            path: rootfs_dir.to_path_buf(),
        })
    } else {
        let builder = RootfsBuilder::new();
        builder.prepare(rootfs_dir.to_path_buf(), base_image).await
    }
}

async fn extract_env_from_image(
    image: &crate::images::ImageObject,
) -> BoxliteResult<Vec<(String, String)>> {
    let image_config = image.load_config().await?;

    let env: Vec<(String, String)> = if let Some(config) = image_config.config() {
        if let Some(envs) = config.env() {
            envs.iter()
                .filter_map(|e| {
                    let parts: Vec<&str> = e.splitn(2, '=').collect();
                    if parts.len() == 2 {
                        Some((parts[0].to_string(), parts[1].to_string()))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    Ok(env)
}
