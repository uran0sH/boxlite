//! Runtime-level metrics (aggregate across all boxes).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Storage for runtime-wide metrics.
///
/// Stored in `RuntimeState`, shared across all operations.
/// All counters are monotonic (never decrease).
#[derive(Clone, Default)]
pub struct RuntimeMetricsStorage {
    /// Total boxes created since runtime startup
    pub(crate) boxes_created: Arc<AtomicU64>,
    /// Total boxes that failed to start
    pub(crate) boxes_failed: Arc<AtomicU64>,
    /// Total commands executed across all boxes
    pub(crate) total_commands: Arc<AtomicU64>,
    /// Total command execution errors across all boxes
    pub(crate) total_exec_errors: Arc<AtomicU64>,
}

impl RuntimeMetricsStorage {
    /// Create new runtime metrics storage.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Handle for querying runtime-wide metrics.
///
/// Cloneable, lightweight handle (only Arc pointers).
/// All counters are monotonic and never reset.
#[derive(Clone)]
pub struct RuntimeMetrics {
    storage: RuntimeMetricsStorage,
}

impl RuntimeMetrics {
    /// Create new handle from storage.
    pub(crate) fn new(storage: RuntimeMetricsStorage) -> Self {
        Self { storage }
    }

    /// Total number of boxes created since runtime startup.
    ///
    /// Incremented when `BoxliteRuntime::create()` is called.
    /// Never decreases (monotonic counter).
    pub fn boxes_created_total(&self) -> u64 {
        self.storage.boxes_created.load(Ordering::Relaxed)
    }

    /// Total number of boxes that failed to start.
    ///
    /// Incremented when box creation or initialization fails.
    /// Never decreases (monotonic counter).
    pub fn boxes_failed_total(&self) -> u64 {
        self.storage.boxes_failed.load(Ordering::Relaxed)
    }

    /// Number of currently running boxes.
    ///
    /// Calculated as: boxes_created - boxes_stopped - boxes_failed
    /// Note: This requires tracking boxes_stopped (TODO in Phase 2).
    ///
    /// Current implementation returns 0 (placeholder).
    pub fn num_running_boxes(&self) -> u64 {
        // TODO: Need to track boxes_stopped counter
        // For now, return 0 as placeholder
        0
    }

    /// Total commands executed across all boxes.
    ///
    /// Incremented on every `LiteBox::exec()` call.
    /// Never decreases (monotonic counter).
    pub fn total_commands_executed(&self) -> u64 {
        self.storage.total_commands.load(Ordering::Relaxed)
    }

    /// Total command execution errors across all boxes.
    ///
    /// Incremented when `LiteBox::exec()` returns error.
    /// Never decreases (monotonic counter).
    pub fn total_exec_errors(&self) -> u64 {
        self.storage.total_exec_errors.load(Ordering::Relaxed)
    }
}
