//! Guest service implementation.
//!
//! Handles guest initialization and management (Init, Ping, Shutdown RPCs).

use crate::service::server::GuestServer;
use boxlite_shared::{
    guest_init_response, rootfs_init, Guest as GuestService, GuestInitError, GuestInitRequest,
    GuestInitResponse, GuestInitSuccess, PingRequest, PingResponse, ShutdownRequest,
    ShutdownResponse,
};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

#[tonic::async_trait]
impl GuestService for GuestServer {
    /// Initialize guest environment.
    ///
    /// This must be called first after connection. It:
    /// 1. Mounts all volumes (virtiofs + block devices)
    /// 2. Sets up rootfs (merged or overlayfs)
    /// 3. Configures network (if specified)
    async fn init(
        &self,
        request: Request<GuestInitRequest>,
    ) -> Result<Response<GuestInitResponse>, Status> {
        let req = request.into_inner();
        info!("Received guest init request");

        // Check if already initialized
        let mut init_state = self.init_state.lock().await;
        if init_state.initialized {
            error!("Guest already initialized (Init can only be called once)");
            return Ok(Response::new(GuestInitResponse {
                result: Some(guest_init_response::Result::Error(GuestInitError {
                    reason: "Guest already initialized (Init can only be called once)".to_string(),
                })),
            }));
        }

        // Step 1: Mount all volumes (virtiofs + block devices)
        info!("Mounting {} volumes", req.volumes.len());
        if let Err(e) = crate::storage::mount_volumes(&req.volumes) {
            error!("Failed to mount volumes: {}", e);
            return Ok(Response::new(GuestInitResponse {
                result: Some(guest_init_response::Result::Error(GuestInitError {
                    reason: format!("Failed to mount volumes: {}", e),
                })),
            }));
        }

        // Step 2: Setup rootfs based on strategy
        let rootfs_path = match req.rootfs {
            Some(rootfs_init) => match rootfs_init.strategy {
                Some(rootfs_init::Strategy::Merged(merged)) => {
                    // Merged rootfs: already mounted via volumes, just record the path
                    info!("Using merged rootfs at {}", merged.path);
                    merged.path
                }
                Some(rootfs_init::Strategy::Overlay(overlay)) => {
                    // Overlay: layers already mounted via volumes, create overlayfs
                    info!(
                        "Creating overlayfs: {} lower dirs -> {} (copy_layers={})",
                        overlay.lower_dirs.len(),
                        overlay.merged_dir,
                        overlay.copy_layers
                    );

                    // If copy_layers is true, copy lower dirs to disk first
                    let lower_dirs = if overlay.copy_layers {
                        match crate::storage::copy_layers_to_disk(
                            &overlay.lower_dirs,
                            &overlay.upper_dir,
                        ) {
                            Ok(copied_dirs) => copied_dirs,
                            Err(e) => {
                                error!("Failed to copy layers to disk: {}", e);
                                return Ok(Response::new(GuestInitResponse {
                                    result: Some(guest_init_response::Result::Error(
                                        GuestInitError {
                                            reason: format!("Failed to copy layers to disk: {}", e),
                                        },
                                    )),
                                }));
                            }
                        }
                    } else {
                        overlay.lower_dirs.clone()
                    };

                    if let Err(e) = crate::overlayfs::mount_overlayfs_direct(
                        &lower_dirs,
                        &overlay.upper_dir,
                        &overlay.work_dir,
                        &overlay.merged_dir,
                    ) {
                        error!("Failed to mount overlayfs: {}", e);
                        return Ok(Response::new(GuestInitResponse {
                            result: Some(guest_init_response::Result::Error(GuestInitError {
                                reason: format!("Failed to mount overlayfs: {}", e),
                            })),
                        }));
                    }

                    overlay.merged_dir
                }
                None => {
                    error!("Missing rootfs strategy in init request");
                    return Ok(Response::new(GuestInitResponse {
                        result: Some(guest_init_response::Result::Error(GuestInitError {
                            reason: "Missing rootfs strategy in init request".to_string(),
                        })),
                    }));
                }
            },
            None => {
                error!("Missing rootfs configuration in init request");
                return Ok(Response::new(GuestInitResponse {
                    result: Some(guest_init_response::Result::Error(GuestInitError {
                        reason: "Missing rootfs configuration in init request".to_string(),
                    })),
                }));
            }
        };

        // Step 3: Configure network (if specified)
        if let Some(network) = req.network {
            info!("Configuring network interface: {}", network.interface);
            if let Err(e) = crate::network::configure_network_from_config(
                &network.interface,
                network.ip.as_deref(),
                network.gateway.as_deref(),
            )
            .await
            {
                error!("Failed to configure network: {}", e);
                return Ok(Response::new(GuestInitResponse {
                    result: Some(guest_init_response::Result::Error(GuestInitError {
                        reason: format!("Failed to configure network: {}", e),
                    })),
                }));
            }
        }

        // Mark as initialized and store rootfs path
        init_state.initialized = true;
        init_state.rootfs_mount = Some(rootfs_path.clone());

        info!(
            "✅ Guest initialized successfully, rootfs at {}",
            rootfs_path
        );
        Ok(Response::new(GuestInitResponse {
            result: Some(guest_init_response::Result::Success(GuestInitSuccess {})),
        }))
    }

    async fn ping(&self, _request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        debug!("Received ping request");
        Ok(Response::new(PingResponse {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }

    async fn shutdown(
        &self,
        _request: Request<ShutdownRequest>,
    ) -> Result<Response<ShutdownResponse>, Status> {
        info!("Received shutdown request");
        Ok(Response::new(ShutdownResponse {}))
    }
}
