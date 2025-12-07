//! Box metrics collection and aggregation

use super::LiteBox;
use super::init::BoxInner;
use super::lifecycle;
use crate::metrics::BoxMetrics;
use boxlite_shared::errors::BoxliteResult;
use std::sync::atomic::Ordering;

/// Get unified metrics (operational + system + network).
///
/// Returns a snapshot of:
/// - Operational metrics: Commands executed, errors, bytes transferred (monotonic counters)
/// - System metrics: CPU usage, memory usage (current values)
/// - Network metrics: Bandwidth, TCP connections, errors (from network backend)
/// - Timing metrics: Spawn and boot duration
///
/// All operational counters never reset - delta calculation is caller's responsibility.
/// System and network metrics are fetched fresh on every call.
pub(crate) async fn metrics(litebox: &LiteBox) -> BoxliteResult<BoxMetrics> {
    let inner = lifecycle::ensure_ready(litebox).await?;

    // Fetch system metrics from controller
    let (cpu_percent, memory_bytes) = fetch_system_metrics(&inner.controller)?;

    // Fetch network metrics from backend if available
    let (network_bytes_sent, network_bytes_received, network_tcp_connections, network_tcp_errors) =
        fetch_network_metrics(&inner.network_backend);

    // Combine operational (from storage) + system (from controller) + network (from backend)
    Ok(BoxMetrics::from_storage(
        &inner.metrics,
        cpu_percent,
        memory_bytes,
        network_bytes_sent,
        network_bytes_received,
        network_tcp_connections,
        network_tcp_errors,
    ))
}

/// Instrument execution metrics at both box and runtime levels.
pub(super) fn instrument_exec_metrics(litebox: &LiteBox, inner: &BoxInner, is_error: bool) {
    // Level 1: Per-box counter (stored internally in LiteBox, like Tokio's TaskMetrics)
    inner.metrics.increment_commands_executed();
    if is_error {
        inner.metrics.increment_exec_errors();
    }

    // Level 2: Runtime aggregate (lock-free!)
    litebox
        .runtime
        .non_sync_state
        .runtime_metrics
        .total_commands
        .fetch_add(1, Ordering::Relaxed);

    if is_error {
        litebox
            .runtime
            .non_sync_state
            .runtime_metrics
            .total_exec_errors
            .fetch_add(1, Ordering::Relaxed);
    }
}

/// Fetch system metrics from controller.
fn fetch_system_metrics(
    controller: &std::sync::Mutex<Box<dyn crate::vmm::VmmController>>,
) -> BoxliteResult<(Option<f32>, Option<u64>)> {
    let controller = controller.lock().map_err(|e| {
        boxlite_shared::errors::BoxliteError::Internal(format!("controller lock poisoned: {}", e))
    })?;
    let raw = controller.metrics()?;
    Ok((raw.cpu_percent, raw.memory_bytes))
}

/// Fetch network metrics from backend if available.
fn fetch_network_metrics(
    network_backend: &Option<Box<dyn crate::net::NetworkBackend>>,
) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
    if let Some(backend) = network_backend {
        match backend.metrics() {
            Ok(Some(net_metrics)) => (
                Some(net_metrics.bytes_sent),
                Some(net_metrics.bytes_received),
                net_metrics.tcp_connections,
                net_metrics.tcp_connection_errors,
            ),
            _ => (None, None, None, None),
        }
    } else {
        (None, None, None, None)
    }
}
