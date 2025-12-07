//! LiteBox - Individual box lifecycle management
//!
//! Provides lazy initialization and execution capabilities for isolated boxes.
//!
//! ## Architecture
//!
//! This module is organized into focused submodules:
//! - `init`: Initialization orchestration (image pulling, rootfs prep, Box startup)
//! - `lifecycle`: State management (start/stop/destroy/cleanup)
//! - `exec`: Command execution
//! - `metrics`: Metrics collection and aggregation

mod exec;
mod init;
mod lifecycle;
mod metrics;

pub use exec::{BoxCommand, ExecResult, ExecStderr, ExecStdin, ExecStdout, Execution, ExecutionId};

pub(crate) use init::BoxBuilder;

use crate::metrics::BoxMetrics;
use crate::runtime::RuntimeInner;
use crate::{BoxID, BoxInfo};
use boxlite_shared::errors::BoxliteResult;
use std::sync::atomic::AtomicBool;
use tokio::sync::OnceCell;

/// BoxHandle represents a running Box.
///
/// This handle provides access to the Box's execution capabilities
/// and lifecycle management through the universal subprocess architecture.
///
/// Conceptually, it plays the same role that `std::process::Child` does for
/// `std::process::Command`, giving callers control over the spawned execution.
///
/// **Design**: Holds `RuntimeInner` (Arc) for direct access to runtime state.
/// All state operations acquire the lock directly and call manager methods.
///
/// **Lazy Initialization**: Heavy initialization (images pulling, Box startup) is deferred
/// until the first API call that requires the box to be running.
pub struct LiteBox {
    id: BoxID,
    runtime: RuntimeInner,
    inner: OnceCell<init::BoxInner>,
    builder: tokio::sync::Mutex<Option<BoxBuilder>>,
    is_shutdown: AtomicBool,
}

impl LiteBox {
    /// Create a new handle.
    ///
    /// **Internal Use**: Called by BoxliteRuntime::create().
    pub(crate) fn new(id: BoxID, runtime: RuntimeInner, builder: BoxBuilder) -> Self {
        Self {
            id,
            runtime,
            inner: OnceCell::new(),
            builder: tokio::sync::Mutex::new(Some(builder)),
            is_shutdown: AtomicBool::new(false),
        }
    }

    /// Get the unique identifier for this box.
    pub fn id(&self) -> &BoxID {
        &self.id
    }

    /// Get current information about this box.
    pub fn info(&self) -> BoxliteResult<BoxInfo> {
        lifecycle::info(self)
    }

    /// Execute a command and return an Execution handle (NEW API).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(litebox: &boxlite::LiteBox) -> Result<(), Box<dyn std::error::Error>> {
    /// use boxlite::BoxCommand;
    /// use futures::StreamExt;
    ///
    /// let mut execution = litebox.exec(BoxCommand::new("ls").arg("-la")).await?;
    ///
    /// // Read stdout
    /// let mut stdout = execution.stdout.take().unwrap();
    /// while let Some(line) = stdout.next().await {
    ///     println!("{}", line);
    /// }
    ///
    /// let status = execution.wait().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn exec(&self, command: BoxCommand) -> BoxliteResult<Execution> {
        exec::exec(self, command).await
    }

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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use boxlite::LiteBox;
    /// # async fn example(litebox: &LiteBox) -> Result<(), Box<dyn std::error::Error>> {
    /// let metrics = litebox.metrics().await?;
    /// println!("Commands executed: {}", metrics.commands_executed_total());
    /// println!("CPU usage: {:?}%", metrics.cpu_percent());
    /// println!("Memory: {:?} bytes", metrics.memory_bytes());
    /// println!("Boot time: {}ms", metrics.guest_boot_duration_ms().unwrap_or(0));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn metrics(&self) -> BoxliteResult<BoxMetrics> {
        metrics::metrics(self).await
    }

    /// Gracefully shut down the box.
    ///
    /// Stops the Box, cleans up resources, and removes the box directory.
    /// Returns `Ok(true)` if shutdown was performed, `Ok(false)` if already shut down.
    pub async fn shutdown(&self) -> BoxliteResult<bool> {
        lifecycle::shutdown(self).await
    }
}

impl Drop for LiteBox {
    fn drop(&mut self) {
        lifecycle::drop_handler(self)
    }
}

// ============================================================================
// THREAD SAFETY ASSERTIONS
// ============================================================================

// Compile-time assertions to ensure LiteBox is Send + Sync
// This is critical for multithreaded usage (e.g., Python GIL release)
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    let _ = assert_send_sync::<LiteBox>;
};
