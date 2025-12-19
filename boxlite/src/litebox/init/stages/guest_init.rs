//! Stage 6: Guest initialization.
//!
//! Sends init configuration to guest and starts container.
//! Builds guest volumes from volume manager, uses rootfs config from vmm_config stage.

use crate::litebox::init::types::{GuestInput, GuestOutput};
use crate::portal::interfaces::{GuestInitConfig, NetworkInitConfig};
use boxlite_shared::errors::BoxliteResult;

/// Initialize guest and start container.
///
/// - Guest.Init: mounts volumes (built from volume_mgr), configures network
/// - Container.Init: prepares rootfs, creates OCI container
pub async fn run(input: GuestInput) -> BoxliteResult<GuestOutput> {
    let container_id_str = input.container_id.as_str();

    // Build guest volumes from volume manager
    let guest_volumes = input.volume_mgr.build_guest_mounts();

    let guest_init_config = GuestInitConfig {
        volumes: guest_volumes,
        network: Some(NetworkInitConfig {
            interface: "eth0".to_string(),
            ip: Some("192.168.127.2/24".to_string()),
            gateway: Some("192.168.127.1".to_string()),
        }),
    };

    // Step 1: Guest Init (volumes + network)
    tracing::info!("Sending guest initialization request");
    let mut guest_interface = input.guest_session.guest().await?;
    guest_interface.init(guest_init_config).await?;
    tracing::info!("Guest initialized successfully");

    // Step 2: Container Init (rootfs + container config + user volume mounts)
    tracing::info!("Sending container configuration to guest");
    let mut container_interface = input.guest_session.container().await?;
    let container_id = container_interface
        .init(
            container_id_str,
            input.container_config,
            input.rootfs_init,
            input.container_mounts,
        )
        .await?;
    tracing::info!(container_id = %container_id, "Container initialized");

    Ok(GuestOutput {
        container_id,
        guest_session: input.guest_session,
    })
}
