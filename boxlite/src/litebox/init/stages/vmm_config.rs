//! Stage 4: VMM configuration.
//!
//! Builds VMM InstanceSpec from prepared components.
//! Includes COW disk creation for rootfs and guest rootfs.

use crate::disk::{BackingFormat, Disk, DiskFormat, Qcow2Helper};
use crate::litebox::init::types::{
    ConfigOutput, ContainerRootfsPrepResult, VmmConfigInput, resolve_user_volumes,
};
use crate::net::{NetworkBackendConfig, NetworkBackendFactory};
use crate::portal::interfaces::ContainerRootfsInitConfig;
use crate::rootfs::operations::fix_rootfs_permissions;
use crate::runtime::constants::{guest_paths, mount_tags};
use crate::vmm::{Entrypoint, InstanceSpec};
use crate::volumes::{ContainerVolumeManager, GuestVolumeManager};
use boxlite_shared::errors::BoxliteResult;
use boxlite_shared::layout::SharedContainerLayout;
use boxlite_shared::{BoxliteError, Transport};
use std::collections::{HashMap, HashSet};

/// Build box configuration.
///
/// Centralizes all ContainerRootfsPrepResult handling to build:
/// - VMM config (InstanceSpec with fs_shares, block_devices)
/// - Guest volumes (for Guest.Init)
/// - Rootfs init config (for Container.Init)
pub async fn run(input: VmmConfigInput<'_>) -> BoxliteResult<ConfigOutput> {
    // Transport setup
    let transport = Transport::unix(input.layout.socket_path());
    let ready_transport = Transport::unix(input.layout.ready_socket_path());

    let user_volumes = resolve_user_volumes(&input.options.volumes)?;

    // Prepare container directories (image/, rw/, rootfs/)
    let container_layout = input
        .layout
        .shared_layout()
        .container(input.container_id.as_str());
    container_layout.prepare()?;

    // Create GuestVolumeManager and configure based on rootfs strategy
    let mut volume_mgr = GuestVolumeManager::new();

    // SHARED virtiofs - needed by all strategies
    // None = guest determines mount path based on tag
    volume_mgr.add_fs_share(
        mount_tags::SHARED,
        input.layout.shared_dir(),
        None,
        false,
        None,
    );

    // Create disks and configure volumes based on rootfs strategy
    let (disk, rootfs_init) = create_container_rootfs_disk(
        &input.rootfs.rootfs_result,
        input.layout,
        &container_layout,
        input.container_id.as_str(),
        &mut volume_mgr,
    )?;

    // Add user volumes via ContainerVolumeManager
    // Host doesn't know guest paths - guest constructs from convention + container_id + volume_name
    let mut container_mgr = ContainerVolumeManager::new(&mut volume_mgr);
    for vol in &user_volumes {
        // Use tag as volume_name for convention-based path
        // guest_path is the user-specified container mount point
        container_mgr.add_volume(
            input.container_id.as_str(), // container_id for convention-based paths
            &vol.tag,                    // volume_name (used by guest to construct path)
            &vol.tag,                    // virtiofs tag (same as volume_name)
            vol.host_path.clone(),
            &vol.guest_path, // container destination path
            vol.read_only,
        );
    }
    let container_mounts = container_mgr.build_container_mounts();

    // Create COW child disk for guest rootfs (protects shared base from writes)
    let (guest_rootfs, init_disk) =
        create_guest_rootfs_disk(input.layout, input.guest_rootfs, &mut volume_mgr)?;

    // Build VMM config from volume manager
    let vmm_config = volume_mgr.build_vmm_config();

    // Guest entrypoint
    let guest_entrypoint =
        build_guest_entrypoint(&transport, &ready_transport, &guest_rootfs, input.options)?;

    // Network backend
    let network_backend = setup_networking(&input.rootfs.container_config, input.options)?;

    // Assemble VMM instance spec
    let box_config = InstanceSpec {
        cpus: input.options.cpus,
        memory_mib: input.options.memory_mib,
        fs_shares: vmm_config.fs_shares,
        block_devices: vmm_config.block_devices,
        guest_entrypoint,
        transport: transport.clone(),
        ready_transport: ready_transport.clone(),
        guest_rootfs,
        network_backend_endpoint: network_backend.as_ref().map(|b| b.endpoint()).transpose()?,
        home_dir: input.home_dir.clone(),
        console_output: None,
    };

    Ok(ConfigOutput {
        box_config,
        network_backend,
        disk,
        init_disk,
        volume_mgr,
        rootfs_init,
        container_mounts,
    })
}

/// Create container rootfs disk based on rootfs strategy.
///
/// Returns the rootfs disk and initialization config.
fn create_container_rootfs_disk(
    rootfs_result: &ContainerRootfsPrepResult,
    layout: &crate::runtime::layout::BoxFilesystemLayout,
    container_layout: &SharedContainerLayout,
    container_id: &str,
    volume_mgr: &mut GuestVolumeManager,
) -> BoxliteResult<(Disk, ContainerRootfsInitConfig)> {
    match rootfs_result {
        ContainerRootfsPrepResult::Merged(_path) => {
            // No disk needed for merged mode - but we require DiskImage mode
            Err(BoxliteError::Internal(
                "Only DiskImage rootfs strategy is supported".into(),
            ))
        }
        ContainerRootfsPrepResult::Layers {
            layers_dir,
            layer_names,
        } => {
            // Mount layers via virtiofs with container_id for convention-based path
            // Guest will mount at: /run/boxlite/shared/containers/{container_id}/layers
            volume_mgr.add_fs_share(
                mount_tags::LAYERS,
                layers_dir.clone(),
                None,
                true,
                Some(container_id.to_string()),
            );

            // Fix permissions
            fix_rootfs_permissions(container_layout.root())?;

            let _rootfs_init = ContainerRootfsInitConfig::Overlay {
                layer_names: layer_names.clone(),
                copy_layers: true,
            };

            // No disk for Layers mode - but we require DiskImage mode
            Err(BoxliteError::Internal(
                "Only DiskImage rootfs strategy is supported".into(),
            ))
        }
        ContainerRootfsPrepResult::DiskImage {
            base_disk_path,
            disk_size,
        } => {
            // Create COW overlay disk for rootfs
            let qcow2_helper = Qcow2Helper::new();
            let disk_path = layout.disk_path();
            let disk = qcow2_helper.create_cow_child_disk(
                base_disk_path,
                BackingFormat::Raw,
                &disk_path,
                *disk_size,
            )?;
            tracing::info!(
                rootfs_disk = %disk.path().display(),
                base_disk = %base_disk_path.display(),
                "Created rootfs COW overlay"
            );

            // Add rootfs block device - guest will mount at its own path
            let rootfs_device =
                volume_mgr.add_block_device(disk.path(), DiskFormat::Qcow2, false, None);

            let rootfs_init = ContainerRootfsInitConfig::DiskImage {
                device: rootfs_device,
            };

            Ok((disk, rootfs_init))
        }
    }
}

fn build_guest_entrypoint(
    transport: &Transport,
    ready_transport: &Transport,
    guest_rootfs: &crate::runtime::guest_rootfs::GuestRootfs,
    options: &crate::runtime::options::BoxOptions,
) -> BoxliteResult<Entrypoint> {
    let listen_uri = transport.to_uri();
    let ready_notify_uri = ready_transport.to_uri();

    // Start with guest rootfs env
    let mut env: Vec<(String, String)> = guest_rootfs.env.clone();

    // Override with user env vars
    for (key, value) in &options.env {
        env.retain(|(k, _)| k != key);
        env.push((key.clone(), value.clone()));
    }

    // Inject RUST_LOG from host
    if !env.iter().any(|(k, _)| k == "RUST_LOG")
        && let Ok(rust_log) = std::env::var("RUST_LOG")
        && !rust_log.is_empty()
    {
        env.push(("RUST_LOG".to_string(), rust_log));
    }

    Ok(Entrypoint {
        executable: format!("{}/boxlite-guest", guest_paths::BIN_DIR),
        args: vec![
            "--listen".to_string(),
            listen_uri,
            "--notify".to_string(),
            ready_notify_uri,
        ],
        env,
    })
}

fn setup_networking(
    container_config: &crate::images::ContainerConfig,
    options: &crate::runtime::options::BoxOptions,
) -> BoxliteResult<Option<Box<dyn crate::net::NetworkBackend>>> {
    let mut port_map: HashMap<u16, u16> = HashMap::new();

    // Step 1: Collect guest ports that user wants to customize
    // User-provided mappings should override image defaults for the same guest port
    let user_guest_ports: HashSet<u16> = options.ports.iter().map(|p| p.guest_port).collect();

    // Step 2: Image exposed ports (only add default 1:1 mapping if user didn't override)
    for port in container_config.tcp_ports() {
        if !user_guest_ports.contains(&port) {
            port_map.insert(port, port);
        }
    }

    // Step 3: User-provided mappings (always applied)
    for port in &options.ports {
        let host_port = port.host_port.unwrap_or(port.guest_port);
        port_map.insert(host_port, port.guest_port);
    }

    let final_mappings: Vec<(u16, u16)> = port_map.into_iter().collect();

    if !final_mappings.is_empty() {
        tracing::info!(
            "Port mappings: {} (image: {}, user: {}, overridden: {})",
            final_mappings.len(),
            container_config.exposed_ports.len(),
            options.ports.len(),
            user_guest_ports
                .intersection(&container_config.tcp_ports().into_iter().collect())
                .count()
        );
    }

    let config = NetworkBackendConfig::new(final_mappings);
    NetworkBackendFactory::create(config)
}

/// Create COW child disk for guest rootfs.
///
/// Protects the shared base guest rootfs from writes by creating a per-box
/// qcow2 overlay. Returns the updated GuestRootfs with device path and the
/// COW disk (to prevent cleanup on drop).
fn create_guest_rootfs_disk(
    layout: &crate::runtime::layout::BoxFilesystemLayout,
    guest_rootfs: &crate::runtime::guest_rootfs::GuestRootfs,
    volume_mgr: &mut GuestVolumeManager,
) -> BoxliteResult<(
    crate::runtime::guest_rootfs::GuestRootfs,
    Option<crate::disk::Disk>,
)> {
    let mut guest_rootfs = guest_rootfs.clone();

    let guest_rootfs_disk =
        if let crate::runtime::guest_rootfs::Strategy::Disk { ref disk_path, .. } =
            guest_rootfs.strategy
        {
            let base_disk_path = disk_path;

            // Get base disk size
            let base_size = std::fs::metadata(base_disk_path)
                .map(|m| m.len())
                .unwrap_or(512 * 1024 * 1024);

            // Create COW child disk
            let guest_rootfs_disk_path = layout.root().join("guest-rootfs.qcow2");
            let qcow2_helper = Qcow2Helper::new();
            let disk = qcow2_helper.create_cow_child_disk(
                base_disk_path,
                BackingFormat::Raw,
                &guest_rootfs_disk_path,
                base_size,
            )?;

            // Register COW child (not the base) via volume manager
            let device_path =
                volume_mgr.add_block_device(disk.path(), DiskFormat::Qcow2, false, None);

            // Update strategy with COW child disk path and device
            guest_rootfs.strategy = crate::runtime::guest_rootfs::Strategy::Disk {
                disk_path: guest_rootfs_disk_path,
                device_path: Some(device_path),
            };

            Some(disk)
        } else {
            None
        };

    Ok((guest_rootfs, guest_rootfs_disk))
}
