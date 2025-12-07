//! Container service interface.

use boxlite_shared::{
    BoxliteError, BoxliteResult, ContainerClient, ContainerConfig as ProtoContainerConfig,
    ContainerInitRequest, container_init_response,
};
use tonic::transport::Channel;

/// Container service interface.
pub struct ContainerInterface {
    client: ContainerClient<Channel>,
}

impl ContainerInterface {
    /// Create from a channel.
    pub fn new(channel: Channel) -> Self {
        Self {
            client: ContainerClient::new(channel),
        }
    }

    /// Initialize container with configuration.
    ///
    /// # Arguments
    /// * `config` - Image-derived container config (entrypoint, env, workdir)
    /// * `state_root` - OCI container state directory path in guest
    /// * `bundle_root` - OCI container bundle directory path in guest
    ///
    /// # Returns
    /// Container ID assigned by the guest
    pub async fn init(
        &mut self,
        config: crate::images::ContainerConfig,
        state_root: &str,
        bundle_root: &str,
    ) -> BoxliteResult<String> {
        let proto_config = ProtoContainerConfig {
            entrypoint: config.cmd.clone(),
            env: config.env.clone(),
            workdir: config.working_dir.clone(),
        };

        tracing::debug!("Sending ContainerInit request");
        tracing::trace!(
            entrypoint = ?config.cmd,
            workdir = %config.working_dir,
            env_count = config.env.len(),
            state_root = %state_root,
            bundle_root = %bundle_root,
            "Container configuration"
        );

        let request = ContainerInitRequest {
            container_config: Some(proto_config),
            state_root: state_root.to_string(),
            bundle_root: bundle_root.to_string(),
        };

        let response = self.client.init(request).await?.into_inner();

        match response.result {
            Some(container_init_response::Result::Success(success)) => {
                tracing::debug!(container_id = %success.container_id, "Container initialized");
                Ok(success.container_id)
            }
            Some(container_init_response::Result::Error(err)) => {
                tracing::error!("Container init failed: {}", err.reason);
                Err(BoxliteError::Internal(format!(
                    "Container init failed: {}",
                    err.reason
                )))
            }
            None => Err(BoxliteError::Internal(
                "ContainerInit response missing result".to_string(),
            )),
        }
    }
}
