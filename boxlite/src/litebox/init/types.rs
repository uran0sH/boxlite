//! Type definitions for initialization pipeline.

use crate::BoxID;
use crate::controller::ShimController;
use crate::images::{ContainerConfig, ImageObject};
use crate::metrics::BoxMetricsStorage;
use crate::net::NetworkBackend;
use crate::portal::GuestSession;
use crate::runtime::RuntimeInner;
use crate::runtime::initrf::InitRootfs;
use crate::runtime::layout::BoxFilesystemLayout;
use crate::runtime::options::BoxOptions;
use crate::vmm::VmmController;
use crate::volumes::Disk;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Switch between merged and overlayfs rootfs strategies.
/// - true: overlayfs (allows COW writes, keeps layers separate)
/// - false: merged rootfs (all layers merged on host)
pub const USE_OVERLAYFS: bool = true;

/// Result of rootfs preparation - either merged or separate layers.
#[derive(Debug)]
pub enum RootfsPrepResult {
    /// Single merged directory (all layers merged on host)
    #[allow(dead_code)]
    Merged(PathBuf),
    /// Layers for guest-side overlayfs
    Layers {
        /// Parent directory containing all extracted layers (mount as single virtiofs share)
        layers_dir: PathBuf,
        /// Subdirectory names for each layer (e.g., "sha256-xxxx")
        layer_names: Vec<String>,
    },
}

/// Final initialized box state.
pub(crate) struct BoxInner {
    pub(in crate::litebox) box_home: PathBuf,
    pub(in crate::litebox) controller: std::sync::Mutex<Box<dyn VmmController>>,
    pub(in crate::litebox) guest_session: GuestSession,
    pub(in crate::litebox) network_backend: Option<Box<dyn NetworkBackend>>,
    /// Per-box operational metrics (stored internally, like Tokio's TaskMetrics)
    pub(in crate::litebox) metrics: BoxMetricsStorage,
    /// RAII-managed disk (auto-cleanup on drop unless installed as disk image)
    pub(in crate::litebox) disk: Disk,
    /// Image object for disk image installation on shutdown
    /// None if this was a COW child (disk image already exists)
    pub(in crate::litebox) image_for_disk_install: Option<ImageObject>,
    /// Container ID for exec requests (used in BOXLITE_EXECUTOR env var)
    pub(in crate::litebox) container_id: String,
}

// ============================================================================
// STAGE INPUT/OUTPUT TYPES
// ============================================================================

/// Input for filesystem stage.
pub struct FilesystemInput<'a> {
    pub box_id: &'a BoxID,
    pub runtime: &'a RuntimeInner,
}

/// Output from filesystem stage.
pub struct FilesystemOutput {
    pub layout: BoxFilesystemLayout,
}

/// Input for rootfs stage.
/// Note: No layout dependency - runs in parallel with filesystem stage.
pub struct RootfsInput<'a> {
    pub options: &'a BoxOptions,
    pub runtime: &'a RuntimeInner,
}

/// Output from rootfs stage.
pub struct RootfsOutput {
    pub container_config: ContainerConfig,
    pub rootfs_result: RootfsPrepResult,
    pub image: ImageObject,
}

/// Input for init image stage.
pub struct InitImageInput<'a> {
    pub runtime: &'a RuntimeInner,
    pub init_rootfs_cell: &'a Arc<OnceCell<InitRootfs>>,
}

/// Output from init image stage.
pub struct InitImageOutput {
    pub init_rootfs: InitRootfs,
}

/// Input for config stage.
pub struct ConfigInput<'a> {
    pub options: &'a BoxOptions,
    pub layout: &'a BoxFilesystemLayout,
    pub rootfs: &'a RootfsOutput,
    pub init_rootfs: &'a InitRootfs,
    pub home_dir: &'a PathBuf,
}

/// Output from config stage.
pub struct ConfigOutput {
    pub box_config: crate::vmm::InstanceSpec,
    pub network_backend: Option<Box<dyn NetworkBackend>>,
    pub disk: Disk,
    pub is_cow_child: bool,
}

/// Input for spawn stage.
pub struct SpawnInput<'a> {
    pub box_id: &'a BoxID,
    pub config: &'a crate::vmm::InstanceSpec,
}

/// Output from spawn stage.
pub struct SpawnOutput {
    pub controller: ShimController,
    pub guest_session: GuestSession,
}

/// Input for guest initialization stage.
pub struct GuestInput {
    pub guest_session: GuestSession,
    pub rootfs_result: RootfsPrepResult,
    pub container_config: ContainerConfig,
    pub is_cow_child: bool,
}

/// Output from guest initialization stage.
pub struct GuestOutput {
    pub container_id: String,
    pub guest_session: GuestSession,
}
