#![cfg(target_os = "linux")]
//! Container service implementation.
//!
//! Handles OCI container lifecycle (Init RPC).

use crate::service::server::GuestServer;
use boxlite_shared::{
    container_init_response, Container as ContainerService, ContainerInitError,
    ContainerInitRequest, ContainerInitResponse, ContainerInitSuccess,
};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::container::Container;

#[tonic::async_trait]
impl ContainerService for GuestServer {
    async fn init(
        &self,
        request: Request<ContainerInitRequest>,
    ) -> Result<Response<ContainerInitResponse>, Status> {
        let init_req = request.into_inner();
        info!("Received container init request");

        // Check if guest is initialized
        let rootfs_mount = {
            let init_state = self.init_state.lock().await;
            match init_state.rootfs_mount.clone() {
                Some(mount) => mount,
                None => {
                    error!("Guest not initialized (Guest.Init must be called first)");
                    return Ok(Response::new(ContainerInitResponse {
                        result: Some(container_init_response::Result::Error(ContainerInitError {
                            reason: "Guest not initialized (Guest.Init must be called first)"
                                .to_string(),
                        })),
                    }));
                }
            }
        };

        // Generate container ID
        let container_id = uuid::Uuid::new_v4().to_string();

        // Extract container config
        let config = init_req
            .container_config
            .ok_or_else(|| Status::invalid_argument("Missing container_config in Init request"))?;

        // Validate configuration
        if config.entrypoint.is_empty() {
            error!("Invalid container config: entrypoint cannot be empty");
            return Ok(Response::new(ContainerInitResponse {
                result: Some(container_init_response::Result::Error(ContainerInitError {
                    reason: "Invalid container config: entrypoint cannot be empty".to_string(),
                })),
            }));
        }

        info!("🚀 Starting OCI container with received configuration");
        debug!(
            entrypoint = ?config.entrypoint,
            workdir = %config.workdir,
            env_count = config.env.len(),
            rootfs = %rootfs_mount,
            state_root = %init_req.state_root,
            bundle_root = %init_req.bundle_root,
            "Container configuration"
        );

        // Start container with configuration from Init message
        match Container::start(
            &rootfs_mount,
            config.entrypoint,
            config.env,
            &config.workdir,
            &init_req.state_root,
            &init_req.bundle_root,
        ) {
            Ok(container) => {
                // Verify container init process is running
                if !container.is_running() {
                    // Gather diagnostic information
                    let diagnostics = container.diagnose_exit();

                    error!(
                        "Container init process exited immediately after start. Diagnostics: {}",
                        diagnostics
                    );

                    return Ok(Response::new(ContainerInitResponse {
                        result: Some(container_init_response::Result::Error(ContainerInitError {
                            reason: format!(
                                "Container init process exited immediately. {}",
                                diagnostics
                            ),
                        })),
                    }));
                }

                info!(
                    container_id = %container_id,
                    "✅ Container started successfully and ready for exec"
                );

                // Store container in registry
                self.containers.lock().await.insert(
                    container_id.clone(),
                    std::sync::Arc::new(tokio::sync::Mutex::new(container)),
                );

                Ok(Response::new(ContainerInitResponse {
                    result: Some(container_init_response::Result::Success(
                        ContainerInitSuccess { container_id },
                    )),
                }))
            }
            Err(e) => {
                error!("Failed to start container: {}", e);
                Ok(Response::new(ContainerInitResponse {
                    result: Some(container_init_response::Result::Error(ContainerInitError {
                        reason: format!("Failed to start container: {}", e),
                    })),
                }))
            }
        }
    }
}
